// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of exchanges_pane.rs.
use mailcraft_model::FolderKind;
use mailcraft_model::exchanges::{File, Row, Way};

use super::*;

fn row(subject: &str, name: &str, email: &str, folder: &str, kind: Option<FolderKind>) -> Row {
    Row {
        message: None,
        server: None,
        subject: subject.into(),
        name: name.into(),
        email: email.into(),
        when: 0,
        folder: folder.into(),
        kind,
        way: Way::In,
        unread: false,
        clip: false,
        files: Vec::new(),
    }
}

#[test]
fn an_amount_is_written_as_a_swiss_page_writes_it() {
    assert_eq!(money(1260.0, "CHF"), "CHF 1'260.–");
    assert_eq!(money(4820.5, "CHF"), "CHF 4'820.50");
    assert_eq!(money(1_234_567.0, "CHF"), "CHF 1'234'567.–");
    assert_eq!(money(12.0, "CHF"), "CHF 12.–");
    assert_eq!(money(-45.9, "EUR"), "EUR -45.90");
    assert_eq!(money(0.0, ""), "0.–");
    assert!(money(f64::NAN, "CHF").starts_with("CHF"));
}

#[test]
fn the_search_words_are_each_found_in_the_subject_the_person_or_the_folder() {
    let r = row("Re: Offre d'installation", "Claire Martin", "claire@atelier-exemple.example", "Archives 2026", None);
    assert!(matches(&r, ""));
    assert!(matches(&r, "  "));
    assert!(matches(&r, "offre"));
    assert!(matches(&r, "OFFRE claire"));
    assert!(matches(&r, "atelier-exemple"));
    assert!(matches(&r, "2026"));
    assert!(!matches(&r, "offre facture"), "every word must be found");
    assert!(!matches(&r, "zzz"));
}

#[test]
fn a_special_folder_is_named_in_the_interface_language_and_another_by_its_own_name() {
    let r = row("", "", "", "INBOX", Some(FolderKind::Inbox));
    assert_eq!(folder_label(&r), crate::i18n::current().folder(FolderKind::Inbox.default_name()));
    let r = row("", "", "", "Clients", Some(FolderKind::Custom));
    assert_eq!(folder_label(&r), "Clients");
    let r = row("", "", "", "Archives 2026", None);
    assert_eq!(folder_label(&r), "Archives 2026");
}

#[test]
fn the_avatar_has_the_first_and_the_last_initial_or_the_address_s_first_letter() {
    assert_eq!(initials("Claire Martin", "x@y.example"), "CM");
    assert_eq!(initials("Claire Anne Martin", "x@y.example"), "CM");
    assert_eq!(initials("Claire", "x@y.example"), "C");
    assert_eq!(initials("", "marc@y.example"), "M");
    assert_eq!(initials("  ", ""), "");
    assert_eq!(initials("élodie dupont", ""), "ÉD");
}

#[test]
fn a_file_is_sized_when_the_size_is_known_and_typed_when_it_is_not() {
    let known = File { name: "a.pdf".into(), mime: "application/pdf".into(), size: 84_000 };
    assert!(!size_label(&known).is_empty());
    assert_ne!(size_label(&known), "application/pdf");
    let unknown = File { name: "a.pdf".into(), mime: "application/pdf".into(), size: 0 };
    assert_eq!(size_label(&unknown), "application/pdf");
}

#[test]
fn the_way_chips_let_in_what_they_say() {
    use crate::exchanges::Show;
    assert!(Show::All.lets(Way::In) && Show::All.lets(Way::Out));
    assert!(Show::In.lets(Way::In) && !Show::In.lets(Way::Out));
    assert!(Show::Out.lets(Way::Out) && !Show::Out.lets(Way::In));
}

#[test]
fn an_address_of_the_instance_is_given_to_the_host_as_its_path() {
    assert_eq!(instance_path("https://x.example/app/history/Customer/C-1").as_deref(), Some("/app/history/Customer/C-1"));
    assert_eq!(instance_path("http://x.example/app/customer/C-1?tab=a#b").as_deref(), Some("/app/customer/C-1?tab=a#b"));
    assert_eq!(instance_path("https://x.example"), None);
    assert_eq!(instance_path("https://x.example/"), None);
    assert_eq!(instance_path("https://x.example//evil.example/x"), None);
    assert_eq!(instance_path("javascript:alert(1)"), None);
    assert_eq!(instance_path("/app/customer/C-1"), None);
    assert_eq!(instance_path("https://x.example/a\nb"), None);
}
