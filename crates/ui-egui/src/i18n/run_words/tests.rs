// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The tests of `run_words`: the matcher over a table of its own, then the table of the crates' sentences.

use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable as _;
use mailcraft_model::{FolderKind, Mailbox};

use super::super::interface_strings::{Literal, literals_of};
use super::super::{catalog, fmt, t_in};
use super::pattern::{Budget, Part, Pattern};
use super::{Family, Kind, Lang, Row, Table, read_steps, read_with};

/// A table of the matcher's own, with the shapes the crates' sentences have.
static SHAPES: &[Row] = &[
    Row { at: "here", family: Family::Line, english: "Couldn't reach the server: {why}", reasons: &[("why", Family::Why)] },
    Row {
        at: "here",
        family: Family::Line,
        english: "Couldn't send '{subject}': {why}. It is held in the Outbox.",
        reasons: &[("why", Family::Hold)],
    },
    Row { at: "here", family: Family::Line, english: "Couldn't send {n} changes to the server: {why}", reasons: &[("why", Family::Why)] },
    Row { at: "here", family: Family::Line, english: "Couldn't send 1 change to the server: {why}", reasons: &[("why", Family::Why)] },
    Row { at: "here", family: Family::Line, english: "Couldn't move '{subject}' to {folder}", reasons: &[] },
    Row { at: "here", family: Family::Line, english: "Up to date", reasons: &[] },
    Row { at: "here", family: Family::Line, english: "Again: {why}", reasons: &[("why", Family::Line)] },
    Row { at: "here", family: Family::Line, english: "Fork {why} or {doing}", reasons: &[("why", Family::Line), ("doing", Family::Line)] },
    Row { at: "here", family: Family::Why, english: "network: {text}", reasons: &[("text", Family::Inner)] },
    Row { at: "here", family: Family::Why, english: "the server is busy (HTTP {status})", reasons: &[] },
    Row { at: "here", family: Family::Inner, english: "{browser} (the server may not let this page reach it)", reasons: &[] },
    Row { at: "here", family: Family::Hold, english: "the server doesn't accept {addresses}", reasons: &[] },
    Row { at: "here", family: Family::Hold, english: "the server refused it ({kind}): {description}", reasons: &[] },
    Row { at: "here", family: Family::Hold, english: "the message has no recipients", reasons: &[] },
];

fn shapes() -> &'static Table {
    static TABLE: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| Table::new(SHAPES))
}

fn mailbox() -> Mailbox {
    let mut mb = Mailbox::with_local_account("Ada", "ada@example.org");
    let account = mb.accounts[0].id;
    let inbox = mb.folder_of_kind(account, FolderKind::Inbox).unwrap();
    mb.folders.iter_mut().find(|f| f.id == inbox).unwrap().name = "INBOX".into();
    mb
}

/// How `text` reads with the matcher's table, in the pseudo-language (whose every word differs from the English).
fn shaped(family: Family, text: &str) -> Option<super::Reading> {
    read_with(shapes(), Lang::Pseudo, family, text, &mailbox())
}

fn pseudo(english: &str) -> &'static str {
    super::super::pseudo(english)
}

#[test]
fn a_template_splits_into_its_text_and_its_placeholders() {
    let parsed = Pattern::parse("Couldn't send '{subject}': {why}. It is held in the Outbox.").unwrap();
    assert_eq!(
        parsed.parts(),
        [
            Part::Literal("Couldn't send '"),
            Part::Slot("subject"),
            Part::Literal("': "),
            Part::Slot("why"),
            Part::Literal(". It is held in the Outbox.")
        ]
    );
    assert_eq!(parsed.slots().collect::<Vec<_>>(), ["subject", "why"]);
    assert_eq!(Pattern::parse("{browser} (it)").unwrap().parts(), [Part::Slot("browser"), Part::Literal(" (it)")]);
    assert_eq!(Pattern::parse("Up to date").unwrap().parts(), [Part::Literal("Up to date")]);
    // Braces that make no placeholder are text, as the catalogs read them.
    assert_eq!(Pattern::parse("a {b c} {} {{n}}").unwrap().parts(), [Part::Literal("a {b c} {} {"), Part::Slot("n"), Part::Literal("}")]);
    assert_eq!(Pattern::parse("the JMAP session has no `{key}`").unwrap().literal_len(), "the JMAP session has no ``".chars().count());
    // Two placeholders side by side, and no text at all, can't be read.
    assert!(Pattern::parse("{a}{b}").is_err());
    assert!(Pattern::parse("x {a}{b} y").is_err());
    assert!(Pattern::parse("{a}").is_err());
    assert!(Pattern::parse("").is_err());
}

#[test]
fn a_line_reads_as_the_row_it_was_made_from() {
    let reading = shaped(Family::Line, "Couldn't reach the server: network: Failed to fetch (the server may not let this page reach it)").unwrap();
    let inner = fmt(pseudo("{browser} (the server may not let this page reach it)"), &[("browser", "Failed to fetch")]);
    let why = fmt(pseudo("network: {text}"), &[("text", &inner)]);
    assert_eq!(reading.text, fmt(pseudo("Couldn't reach the server: {why}"), &[("why", &why)]));
    assert!(reading.unread.is_empty(), "{:?}", reading.unread);
    assert_eq!(
        reading.rows,
        ["Couldn't reach the server: {why}", "network: {text}", "{browser} (the server may not let this page reach it)"],
        "the rows that read it, the outermost first"
    );
    // A row with no placeholder reads its own text only.
    assert_eq!(shaped(Family::Line, "Up to date").unwrap().text, pseudo("Up to date"));
    assert!(shaped(Family::Line, "Up to date!").is_none());
    assert!(shaped(Family::Line, " Up to date").is_none());
    // A row of another family doesn't read a line.
    assert!(shaped(Family::Line, "network: down").is_none());
    assert!(shaped(Family::Why, "network: down").is_some());
}

#[test]
fn a_reason_no_row_reads_is_kept_as_it_came_and_said_to_be_unread() {
    let reading = shaped(Family::Line, "Couldn't reach the server: the proxy said it would be back after lunch").unwrap();
    assert_eq!(reading.text, fmt(pseudo("Couldn't reach the server: {why}"), &[("why", "the proxy said it would be back after lunch")]));
    assert_eq!(reading.unread, [(Family::Why, "the proxy said it would be back after lunch".to_string())]);
    // And the words under a reason that is read.
    let reading = shaped(Family::Line, "Couldn't reach the server: network: connection reset").unwrap();
    assert_eq!(reading.unread, [(Family::Inner, "connection reset".to_string())]);
    let why = fmt(pseudo("network: {text}"), &[("text", "connection reset")]);
    assert_eq!(reading.text, fmt(pseudo("Couldn't reach the server: {why}"), &[("why", &why)]));
    // A line whose frame no row reads is not read at all.
    assert!(shaped(Family::Line, "Receiving mail failed: the server closed the connection").is_none());
}

#[test]
fn a_number_is_digits_and_a_type_word_is_letters_and_digits() {
    assert!(shaped(Family::Line, "Couldn't send 12 changes to the server: network: x").is_some());
    let one = shaped(Family::Line, "Couldn't send 1 change to the server: network: x").unwrap();
    assert_eq!(one.rows.first(), Some(&"Couldn't send 1 change to the server: {why}"));
    for not_a_number in ["twelve", "1 2", "-3", "3.5", "", "١٢"] {
        assert!(shaped(Family::Line, &format!("Couldn't send {not_a_number} changes to the server: network: x")).is_none(), "{not_a_number:?}");
    }
    assert!(shaped(Family::Line, &format!("Couldn't send {} changes to the server: network: x", "9".repeat(21))).is_none());
    // A subject is told from a count by the quote after `send `.
    let held = shaped(Family::Line, "Couldn't send '12 changes': the message has no recipients. It is held in the Outbox.").unwrap();
    assert_eq!(held.rows.first(), Some(&"Couldn't send '{subject}': {why}. It is held in the Outbox."));
    assert!(shaped(Family::Hold, "the server refused it (serverFail): it said so").is_some());
    for not_a_kind in ["server fail", "server-fail", "", &"x".repeat(41), "sérverFail"] {
        assert!(shaped(Family::Hold, &format!("the server refused it ({not_a_kind}): it said so")).is_none(), "{not_a_kind:?}");
    }
}

#[test]
fn a_subject_that_holds_the_text_after_it_is_split_where_the_reason_reads() {
    let frame = pseudo("Couldn't send '{subject}': {why}. It is held in the Outbox.");
    // The subject holds `': `: the reading that reads the reason wins over the earliest place.
    let reading = shaped(Family::Line, "Couldn't send 'Re: it's': the plan': the message has no recipients. It is held in the Outbox.").unwrap();
    assert!(reading.unread.is_empty(), "{:?}", reading.unread);
    assert_eq!(reading.text, fmt(frame, &[("subject", "Re: it's': the plan"), ("why", pseudo("the message has no recipients"))]));
    // When no place reads the reason (a server's words), the earliest is taken.
    let reading = shaped(Family::Line, "Couldn't send 'A': b': it fell over. It is held in the Outbox.").unwrap();
    assert_eq!(reading.unread, [(Family::Hold, "b': it fell over".to_string())]);
    assert_eq!(reading.text, fmt(frame, &[("subject", "A"), ("why", "b': it fell over")]));
    // The last text of a row ends the line, whatever the reason holds of it.
    let reading =
        shaped(Family::Line, "Couldn't send 'A': the server refused it (x1): It is held in the Outbox.. It is held in the Outbox.").unwrap();
    assert!(reading.unread.is_empty(), "{:?}", reading.unread);
    let why = fmt(pseudo("the server refused it ({kind}): {description}"), &[("kind", "x1"), ("description", "It is held in the Outbox.")]);
    assert_eq!(reading.text, fmt(frame, &[("subject", "A"), ("why", &why)]));
}

#[test]
fn values_are_shown_as_they_came_and_never_filled_again() {
    let frame = pseudo("Couldn't send '{subject}': {why}. It is held in the Outbox.");
    // A subject or a server's words that hold a placeholder are text.
    let reading = shaped(Family::Line, "Couldn't send '{why}': the server refused it (k): {subject} {kind}. It is held in the Outbox.").unwrap();
    let why = fmt(pseudo("the server refused it ({kind}): {description}"), &[("kind", "k"), ("description", "{subject} {kind}")]);
    assert_eq!(reading.text, fmt(frame, &[("subject", "{why}"), ("why", &why)]));
    // The engine's two words are the interface's; everything else of a subject or an address list is as it came.
    let reading =
        shaped(Family::Line, "Couldn't send '(no subject)': the server doesn't accept (address), (address). It is held in the Outbox.").unwrap();
    let why = fmt(pseudo("the server doesn't accept {addresses}"), &[("addresses", &format!("{a}, {a}", a = pseudo("(address)")))]);
    assert_eq!(reading.text, fmt(frame, &[("subject", pseudo("(no subject)")), ("why", &why)]));
    let reading =
        shaped(Family::Line, "Couldn't send 'Fwd: (no subject) from (address)': the message has no recipients. It is held in the Outbox.").unwrap();
    let subject = format!("Fwd: (no subject) from {}", pseudo("(address)"));
    assert_eq!(reading.text, fmt(frame, &[("subject", &subject), ("why", pseudo("the message has no recipients"))]));
    // A standard folder by the name the folder pane gives it, any other by its own.
    let moved = pseudo("Couldn't move '{subject}' to {folder}");
    assert_eq!(shaped(Family::Line, "Couldn't move 'Hi' to INBOX").unwrap().text, fmt(moved, &[("subject", "Hi"), ("folder", pseudo("Inbox"))]));
    assert_eq!(shaped(Family::Line, "Couldn't move 'Hi' to Projects").unwrap().text, fmt(moved, &[("subject", "Hi"), ("folder", "Projects")]));
}

#[test]
fn a_line_in_english_reads_as_itself() {
    let line = "Couldn't send 'Hello': the server doesn't accept (address). It is held in the Outbox.";
    let reading = read_with(shapes(), Lang::English, Family::Line, line, &mailbox()).unwrap();
    assert_eq!(reading.text, line);
    assert!(reading.unread.is_empty());
}

#[test]
fn a_hostile_line_ends_soon_and_is_left_as_it_came() {
    let started = Instant::now();
    // Too long to be a line.
    assert!(shaped(Family::Line, &format!("Couldn't reach the server: {}", "x".repeat(100_000))).is_none());
    // A line that costs more than its budget is left as it came, however well it would read.
    let line = "Couldn't reach the server: network: Failed to fetch (the server may not let this page reach it)";
    assert!(read_steps(shapes(), Lang::Pseudo, Family::Line, line, &mailbox(), 1_000).is_some());
    assert!(read_steps(shapes(), Lang::Pseudo, Family::Line, line, &mailbox(), 5).is_none());
    // Floods of the text a placeholder ends at, in rows that read a line inside a line.
    let floods = [
        format!("{}Up to date{}", "Fork ".repeat(60), " or Up to date".repeat(60)),
        format!("Couldn't send '{}': x. It is held in the Outbox.", "': ".repeat(300)),
        format!("Fork {} or {}", "Fork Up to date or ".repeat(25), "Up to date or ".repeat(25)),
    ];
    for flood in &floods {
        let _ = shaped(Family::Line, flood);
    }
    // Deep: a reason inside a reason inside a reason… is read four deep, and the rest is the innermost's words.
    let deep = shaped(Family::Line, "Again: Again: Again: Again: Again: Up to date").unwrap();
    assert_eq!(deep.rows.len(), 4, "{:?}", deep.rows);
    assert_eq!(deep.unread, [(Family::Line, "Again: Up to date".to_string())]);
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn the_budget_is_spent_once_and_for_all() {
    let budget = Budget::new(2);
    assert!(budget.take() && budget.take());
    assert!(!budget.spent());
    assert!(!budget.take());
    assert!(budget.spent() && !budget.take());
}

#[test]
fn junk_never_makes_it_panic() {
    let junk: Vec<String> = [
        "",
        "{",
        "}",
        "{}",
        "{why}",
        "'",
        "''",
        "': ",
        ". It is held in the Outbox.",
        "Couldn't send '",
        "Couldn't send ''",
        "Couldn't send '': . It is held in the Outbox.",
        "Couldn't send '☃': ☃. It is held in the Outbox.",
        "Couldn't reach the server: ",
        "Couldn't reach the server:",
        "network: ",
        " (the server may not let this page reach it)",
        "\u{0}",
        "\u{202e}Couldn't reach the server: x",
        "Couldn't reach the server: \u{301}",
        "Couldn't send 1 change to the server: \n",
        "e\u{301}\u{301}\u{301}",
        "Couldn't move '' to ",
        "Couldn't move 'a' to \u{0}",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .chain(["'".repeat(1_000), "{why}".repeat(200), "Again: ".repeat(140), "☃".repeat(300), format!("Couldn't send '{}", "é".repeat(500))])
    .collect();
    for text in &junk {
        for family in Family::ALL {
            for lang in [Lang::English, Lang::Pseudo, Lang::from_tag("fr").unwrap()] {
                let _ = read_with(shapes(), lang, family, text, &mailbox());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The table of the crates' sentences

/// The French catalog.
fn french() -> Lang {
    Lang::from_tag("fr").unwrap()
}

/// A value of the kind a placeholder holds, for a sample of its row; a reason is a sentence of its family.
fn sample_value(row: &Row, name: &str, depth: usize) -> String {
    match Kind::of(name) {
        Some(Kind::Number) => "3".into(),
        Some(Kind::Token) => "serverFail".into(),
        Some(Kind::Subject) => "Invoice 42".into(),
        Some(Kind::Addresses) => "bob@example.org".into(),
        Some(Kind::Folder) => "Projects".into(),
        Some(Kind::Text) => "it said so".into(),
        Some(Kind::Reason) => {
            let family = row.reason(name).unwrap_or(row.family);
            // The family's first row that holds no reason of its own, filled in turn.
            let inner = super::rows()
                .iter()
                .find(|r| r.family == family && Pattern::parse(r.english).is_ok_and(|p| p.slots().all(|s| Kind::of(s) != Some(Kind::Reason))))
                .unwrap_or_else(|| panic!("{family:?} has a row with no reason in it"));
            sample(inner, depth + 1)
        }
        None => panic!("{:?}: {name:?} is a placeholder nobody knows the kind of", row.english),
    }
}

/// The row's sentence filled with sample values.
fn sample(row: &Row, depth: usize) -> String {
    let pattern = Pattern::parse(row.english).unwrap();
    let values: Vec<(&str, String)> = pattern.slots().map(|name| (name, sample_value(row, name, depth))).collect();
    let args: Vec<(&str, &str)> = values.iter().map(|(name, value)| (*name, value.as_str())).collect();
    fmt(row.english, &args)
}

#[test]
fn every_row_is_a_template_whose_placeholders_are_known() {
    let mut seen = std::collections::HashSet::new();
    for row in super::rows() {
        assert!(seen.insert(row.english), "{:?} is in the table twice", row.english);
        let pattern = Pattern::parse(row.english).unwrap_or_else(|e| panic!("{e}"));
        let slots: Vec<&str> = pattern.slots().collect();
        for name in &slots {
            let kind = Kind::of(name).unwrap_or_else(|| panic!("{:?}: {name:?} is a placeholder nobody knows the kind of", row.english));
            if kind == Kind::Reason {
                assert!(row.reason(name).is_some(), "{:?}: {{{name}}} holds a reason, and the row doesn't say what family it is", row.english);
            }
        }
        for (name, _) in row.reasons {
            assert!(slots.contains(name), "{:?} names a reason {{{name}}} it doesn't have", row.english);
            assert_eq!(Kind::of(name), Some(Kind::Reason), "{:?}: {{{name}}} is not a reason", row.english);
        }
        let mut sorted = slots.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, catalog::placeholders(row.english), "{:?}", row.english);
        assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(row.at).is_file(), "{:?}: {} is no file", row.english, row.at);
    }
}

#[test]
fn every_row_reads_back_its_own_sentence_and_no_other_row_comes_first() {
    let mb = mailbox();
    for row in super::rows() {
        let line = sample(row, 0);
        let reading = super::explain(Lang::Pseudo, row.family, &line, &mb).unwrap_or_else(|| panic!("{line:?} isn't read"));
        assert_eq!(reading.rows.first(), Some(&row.english), "{line:?} reads as another row: {:?}", reading.rows);
        assert!(reading.unread.is_empty(), "{line:?}: {:?}", reading.unread);
        // The pseudo-language changes every row's words: nothing of it is left in English.
        assert_ne!(reading.text, line, "{:?}", row.english);
        // And in English a line is itself.
        assert_eq!(super::explain(Lang::English, row.family, &line, &mb).map(|r| r.text), Some(line.clone()));
        // The values come back as they were.
        for value in ["it said so", "serverFail", "Invoice 42", "bob@example.org", "Projects"] {
            assert_eq!(reading.text.matches(value).count(), line.matches(value).count(), "{line:?} -> {:?}", reading.text);
        }
    }
}

#[test]
fn every_row_has_french_of_its_own() {
    for row in super::rows() {
        let french = t_in(french(), row.english);
        assert_ne!(french, row.english, "{:?} has no French line", row.english);
        assert_eq!(catalog::placeholders(french), catalog::placeholders(row.english), "{:?}", row.english);
    }
    // The engine's two words too.
    assert_eq!(t_in(french(), super::ADDRESS), "(adresse)");
    assert_eq!(t_in(french(), super::NO_SUBJECT), "(sans objet)");
    assert_ne!(t_in(french(), super::NO_REASON), super::NO_REASON);
}

#[test]
fn the_crates_lines_read_in_french() {
    let mb = mailbox();
    let fr = |text: &str| super::explain(french(), Family::Line, text, &mb).map(|r| r.text);
    assert_eq!(
        fr("Couldn't reach the server: network: Failed to fetch (the server may not let this page reach it)").as_deref(),
        Some("Impossible de joindre le serveur : réseau : Failed to fetch (le serveur ne permet peut-être pas à cette page de l’atteindre)")
    );
    assert_eq!(
        fr("Couldn't read the mail in 3 mailboxes: the server refused: serverFail this mailbox can't be read just now").as_deref(),
        Some("Impossible de lire le courrier de 3 dossiers : le serveur a refusé : serverFail this mailbox can't be read just now")
    );
    assert_eq!(
        fr("Couldn't bring the mail down from the server: the server is busy (HTTP 503)").as_deref(),
        Some("Impossible de récupérer le courrier du serveur : le serveur est occupé (HTTP 503)")
    );
    assert_eq!(
        fr("The server failed 3 mailboxes in a row; the rest wait for the next run.").as_deref(),
        Some("Le serveur n’a pas pu traiter 3 dossiers d’affilée ; les autres attendront le prochain Envoyer/Recevoir.")
    );
    // A frame of ours over a reason no row reads: the frame is translated, the reason is kept as it came.
    assert_eq!(
        fr("Couldn't bring the mail down from the server: the proxy said it would be back after lunch").as_deref(),
        Some("Impossible de récupérer le courrier du serveur : the proxy said it would be back after lunch")
    );
    // A line no row reads stays as it came.
    assert_eq!(fr("Receiving mail failed: the server closed the connection"), None);
}

/// A new contact the server has no room for, as the engine says it (`remote_contacts::written`).
#[test]
fn a_new_contact_the_server_has_no_room_for_reads_in_french() {
    let line = "The server's mailbox is full: \"Gus Hale\" waits here and goes as soon as there is room.";
    let read = super::explain(french(), Family::Line, line, &mailbox()).map(|reading| reading.text);
    assert_eq!(read.as_deref(), Some("L’espace du serveur est plein : \"Gus Hale\" attend ici et partira dès qu’il y aura de la place."));
    // And a change or a deletion of one the server has.
    for line in [
        "The server's mailbox is full: the change to \"Gus Hale\" waits here and goes as soon as there is room.",
        "The server's mailbox is full: deleting \"Gus Hale\" waits here and goes as soon as there is room.",
    ] {
        let read = super::explain(french(), Family::Line, line, &mailbox()).expect("read");
        assert!(read.text.starts_with("L’espace du serveur est plein") && read.text.contains("\"Gus Hale\""), "{}", read.text);
        assert!(!read.text.contains("the change to") && !read.text.contains("deleting"), "read whole, by a row of its own: {}", read.text);
    }
}

/// What a JMAP account's calendar and address book job says went wrong, what became of a change of an event or a
/// contact, and the engine's refusals of them: in French, the sentences the engine puts after a line, the words for an
/// item with no name and the parts of an event included.
#[test]
fn the_calendar_and_contact_lines_read_in_french() {
    let mb = mailbox();
    let fr = |text: &str| super::explain(french(), Family::Line, text, &mb).map(|r| r.text);
    let t = |english: &str| t_in(french(), english).to_string();
    // A job's line, round a reason of its own.
    assert_eq!(
        fr("Couldn't list the calendars on the server: the answer has no list of calendars"),
        Some(fmt(&t("Couldn't list the calendars on the server: {why}"), &[("why", &t("the answer has no list of calendars"))]))
    );
    assert_eq!(
        fr("Couldn't read 3 contacts of the address books: the server's answer did not have them"),
        Some(fmt(&t("Couldn't read {n} contacts of the address books: {why}"), &[("n", "3"), ("why", &t("the server's answer did not have them"))]))
    );
    // What became of a change, with the sentence the engine puts after it, read off its end.
    let line = "\"Gus Hale\" was not sent to the server: its address book can't be changed any more. It stays here until this tab is closed: copy what you need of it.";
    let head = fmt(&t("{name} was not sent to the server: its address book can't be changed any more."), &[("name", "\"Gus Hale\"")]);
    assert_eq!(fr(line), Some(format!("{head} {}", t("It stays here until this tab is closed: copy what you need of it."))));
    // Two of them.
    let line = "A contact was made on the server, which doesn't say what it calls it. It is kept on this computer only. The next read brings the server's card too.";
    let read = fr(line).expect("read");
    assert!(!read.contains("It is kept") && !read.contains("The next read") && read.starts_with(&t("A contact")), "{read}");
    // A refusal with the server's own words in brackets: the frame in French, the words as they came.
    let read = fr("The server wouldn't take the change to \"Lunch\" (forbidden: the desk says no). It was put back as it was.").expect("read");
    assert_eq!(
        read,
        fmt(
            &t("The server wouldn't take the change to {name} ({why}). It was put back as it was."),
            &[("name", "\"Lunch\""), ("why", "forbidden: the desk says no")]
        )
    );
    // An event with no name, and what of it the server shows otherwise.
    let read = fr(
        "The server took the change to an event but has its time and its title otherwise than it was sent. It is shown here as the server has it.",
    )
    .expect("read");
    for english in ["an event", "its time", "its title"] {
        assert!(read.contains(&t(english)) && !read.contains(english), "{english}: {read}");
    }
    // The engine's refusals, and what a greyed control says.
    for refusal in [
        "Your rights do not let you delete this contact",
        "This calendar's server keeps no categories, importance, response options or proposed times.",
        "This page keeps no tasks: they would be gone at the next visit",
    ] {
        assert_eq!(fr(refusal), Some(t(refusal)));
        assert_ne!(t(refusal), refusal, "{refusal} has French");
    }
    assert_eq!(
        fr("not available: Your rights do not let you change this contact"),
        Some(fmt(&t("not available: {why}"), &[("why", &t("Your rights do not let you change this contact"))]))
    );
    // A reason after the window's own words.
    crate::i18n::set_current(french());
    let reason = super::reason("the server no longer has the series", &mb).into_owned();
    crate::i18n::set_current(Lang::English);
    assert_eq!(reason, t("the server no longer has the series"));
}

/// A line that only looks as though it ends with one of the engine's sentences is read whole, or left as it came.
#[test]
fn a_line_is_read_off_its_end_only_where_what_comes_before_reads_too() {
    let mb = mailbox();
    let fr = |text: &str| super::explain(french(), Family::Line, text, &mb).map(|r| r.text);
    assert_eq!(fr("Something nobody wrote. It is back in the calendar."), None);
    // Many of them, more than are read off: left as it came.
    let many = format!("The server wouldn't take \"X\".{}", " It is back in the calendar.".repeat(5));
    assert!(fr(&many).is_none_or(|read| read.contains("It is back")), "only so many are read off");
}

// ---------------------------------------------------------------------------------------------
// The census: the table against the crates' source

/// The files whose every sentence a person can be shown has a row (or is on [`NOT_READ`]).
const CENSUS: &[&str] = &[
    "crates/jmap/src/lib.rs",
    "crates/jmap/src/mail.rs",
    "crates/jmap/src/mail/up.rs",
    "crates/jmap/src/url.rs",
    "apps/mailcraft-web/src/fetch.rs",
    "crates/jmap/src/send.rs",
    "crates/jmap/src/mail/outgoing.rs",
    "crates/engine/src/cmd/jmap_send.rs",
    "crates/engine/src/cmd/jmap_up.rs",
    "crates/engine/src/undo.rs",
    "crates/model/src/jmap_server.rs",
    "crates/engine/src/cmd/notes/words.rs",
    "crates/jmap/src/pim/notes.rs",
    "crates/jmap/src/pim.rs",
    "crates/jmap/src/pim/calendars.rs",
    "crates/jmap/src/pim/contacts.rs",
    "crates/jmap/src/pim/push.rs",
    "crates/jmap/src/pim/contacts/push.rs",
    "crates/jmap/src/pim/write.rs",
    "crates/engine/src/cmd/remote_contacts/written.rs",
    "crates/engine/src/cmd/kept_here.rs",
];

/// Sentences of those files that no row reads, and why: they are no line a person is shown, or not one of a JMAP account's.
const NOT_READ: &[(&str, &str)] = &[
    ("sign-in failed: {0}", "a refused sign-in ends the run as `auth_refused`, with no line (the engine says STA1)"),
    ("the user name, password or access token has a character that can't be sent", "an `Error::Auth`, as above"),
    ("the user name, password or access token is too long to be sent", "an `Error::Auth`, as above"),
    ("there is no user name or password to sign in with", "an `Error::Auth`, as above"),
    ("there is no access token to sign in with", "an `Error::Auth`, as above"),
    ("the server refused the credentials", "an `Error::Auth`, as above"),
    ("JMAP: there is no such call", "the callers take only the answers they asked for"),
    ("JMAP: that answer was taken", "as above"),
    ("{} method calls in one request, and the server takes {}", "every caller holds its request to the server's `maxCallsInRequest`"),
    ("not a blob id", "a download's alone, whose problems are only logged (`graph_fetch`)"),
    ("(not a web address)", "the `Debug` of a request"),
    ("Answers({} of which {} failed)", "the `Debug` of the answers"),
    ("the server takes no message as large as this", "`Outgoing::trouble` and `draft_failed` say a refused size in their own words"),
    ("the server's answer to an upload names no blob", "an answer that is no JMAP is said in their own words too"),
    ("the server kept another number of octets than it was sent", "as above"),
    ("the server's answer doesn't say what it made of the message", "as above"),
    ("the server's answer to a filing names no email", "as above"),
    ("the answer has no list of identities", "as above"),
    ("a copy filed in Sent for a message that was not sent", "words for the log, of what `destroy_unwanted` couldn't destroy"),
    ("the draft a message was written as", "as above"),
    ("the earlier copy of a draft", "as above"),
    (
        "The folder '{}' was deleted for good: that can't be undone here.",
        "a folder deleted for good is a Microsoft account's: a JMAP account's folders are its server's",
    ),
    ("{} folders were deleted for good: that can't be undone here.", "as above"),
    (
        "'{}' was moved out of its account and the server's copy removed: that can't be undone here.",
        "a move out of an account is a Microsoft account's: one out of a JMAP account is refused",
    ),
    ("{} messages were moved out of their account and the server's copies removed: that can't be undone here.", "as above"),
    ("the server's answer said nothing of it", "a note change's words, which a line about the note shows as they came"),
    ("the version its words were read at isn't known yet", "as above: a change that waits for a read"),
    (
        "an online meeting is the whole series': it is not asked for one occurrence",
        "for the record: the person is told in words of their own (`remote::unmade_words`)",
    ),
    ("the server no longer makes online meetings", "as above"),
    ("its account is no longer in the mailbox", "why a waiting change of a contact is given up: words for the log"),
    ("the contact it is about is no longer in the contacts", "as above"),
    ("it doesn't match the contact it names", "as above"),
    ("it doesn't say which contact it is about", "as above"),
    ("the contact it deletes is still in the contacts", "as above"),
    ("the contact's address book can't be changed any more", "as above"),
    ("another change about the same contact waits before it", "as above"),
    ("too many changes were waiting", "as above"),
    ("the server kept failing with it", "as above"),
    ("`result` is required", "a command's parameters, for a host or an agent"),
];

/// The text between the placeholders of a template or of a format string (`{}`, `{0}`, `{why}`, `{:?}`), each with text
/// of its own: what a row must have, in order, for the sentence to be its.
fn fragments(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((head, tail)) = rest.split_once('{') {
        out.push(head);
        rest = tail.split_once('}').map_or("", |(_, after)| after);
    }
    out.push(rest);
    out.into_iter().filter(|fragment| fragment.chars().any(char::is_alphanumeric)).collect()
}

/// Whether `english` has every fragment, in order.
fn has_in_order(english: &str, fragments: &[&str]) -> bool {
    let mut rest = english;
    fragments.iter().all(|fragment| match rest.find(fragment) {
        Some(at) => {
            rest = rest.get(at + fragment.len()..).unwrap_or_default();
            true
        }
        None => false,
    })
}

fn source(path: &str) -> String {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
    std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

/// What a crate writes between the pieces of a sentence: digits, spaces and punctuation.
fn between(c: char) -> bool {
    c.is_ascii_digit() || c.is_whitespace() || c.is_ascii_punctuation()
}

/// The longest piece of `literals` that `fragment` is made of, when it is made of them as a crate makes a sentence of
/// pieces (`plural`, `{what}`): with the digits, spaces and punctuation it writes between them. None when it isn't.
fn made_of(fragment: &str, literals: &[Literal]) -> Option<usize> {
    let has = |piece: &str| literals.iter().any(|l| l.text.contains(piece));
    let (mut rest, mut longest) = (fragment, 0);
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(between) {
            rest = after;
            continue;
        }
        // The longest start of what is left that a literal has, in whole words.
        let whole = |end: &usize| rest.get(*end..).is_none_or(|after| !after.starts_with(char::is_alphanumeric));
        let end = rest.char_indices().map(|(at, c)| at + c.len_utf8()).filter(whole).rfind(|end| rest.get(..*end).is_some_and(has))?;
        longest = longest.max(rest.get(..end).map_or(0, |piece| piece.chars().count()));
        rest = rest.get(end..).unwrap_or_default();
    }
    Some(longest)
}

/// Whether a row has `pieces` in order; or one has the first of them and the rest are sentences of [`Family::Tail`], each
/// a row's whole, which the engine writes after a line (`{name} was made… .{kept} The next read…`).
fn read_by_rows(pieces: &[&str]) -> bool {
    let tail = |piece: &&str| {
        let sentence = piece.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end();
        super::rows().iter().any(|row| row.family == Family::Tail && row.english == sentence)
    };
    (0..=pieces.len()).rev().any(|k| {
        let (head, rest) = pieces.split_at(k);
        let head_read = head.is_empty() || super::rows().iter().any(|row| row.family != Family::Tail && has_in_order(row.english, head));
        head_read && rest.iter().all(tail)
    })
}

#[test]
fn every_row_is_written_by_the_file_it_names() {
    for row in super::rows() {
        let literals: Vec<Literal> = literals_of(&source(row.at)).into_iter().filter(|l| !l.logged).collect();
        let mut longest = 0;
        for fragment in fragments(row.english) {
            let made = made_of(fragment, &literals);
            assert!(made.is_some(), "{:?}: {} has no {fragment:?} any more, whole or in pieces (reworded, or moved?)", row.english, row.at);
            longest = longest.max(made.unwrap_or_default());
        }
        // Pieces of more than the few words any file could have (a sentence made of pieces has them as short as `messages`).
        let whole = fragments(row.english).into_iter().map(|f| f.trim_matches(between).chars().count()).max().unwrap_or_default();
        assert!(longest >= whole.min(8), "{:?}: {} has only pieces of it {longest} characters long", row.english, row.at);
    }
}

#[test]
fn every_sentence_of_the_census_has_a_row() {
    let mut not_shown_seen = std::collections::HashSet::new();
    for path in CENSUS {
        for literal in literals_of(&source(path)) {
            let text = literal.text.as_str();
            // A sentence for a person: three words or more, not for the log.
            if literal.logged || text.split_whitespace().count() < 3 || !text.chars().any(char::is_alphabetic) {
                continue;
            }
            if super::WORDS.contains(&text) {
                continue;
            }
            if NOT_READ.iter().any(|(sentence, _)| *sentence == text) {
                not_shown_seen.insert(text.to_string());
                continue;
            }
            let pieces = fragments(text);
            assert!(
                read_by_rows(&pieces),
                "{path}:{}: {text:?} has no row in run_words (a sentence the crate added or reworded): add one, or say in NOT_READ why no row reads it",
                literal.line
            );
        }
    }
    for (sentence, _) in NOT_READ {
        assert!(not_shown_seen.contains(*sentence), "{sentence:?} is in NOT_READ and in none of the census's files any more");
    }
}

// ---------------------------------------------------------------------------------------------
// Where the window draws them

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A window with a JMAP account, whose problems the tests keep as a run would.
fn window() -> (crate::MailApp, mailcraft_model::Id) {
    let mut app = crate::MailApp::new(mailcraft_engine::Session::at(Mailbox::with_local_account("", ""), NOW, 0), crate::Services::default());
    app.autosave = false;
    app.auto_sync = false;
    let added = app.execute(
        "account.add",
        serde_json::json!({"email": "ada@example.org", "kind": "jmap", "jmap": {"session": "https://jmap.example.org/jmap/session", "bearer": true}}),
    );
    let account = added.unwrap()["id"].as_u64().unwrap();
    (app, account)
}

/// A failed run's lines, and their French.
const FAILED: [(&str, &str); 3] = [
    ("Couldn't reach the server: network: connection refused", "Impossible de joindre le serveur : réseau : connection refused"),
    (
        "Couldn't read the mail in 1 mailbox: the server is busy (HTTP 429)",
        "Impossible de lire le courrier de 1 dossier : le serveur est occupé (HTTP 429)",
    ),
    (
        "The server is busy. What waits to go will go with a later Send/Receive.",
        "Le serveur est occupé. Ce qui attend d’être envoyé partira avec un Envoyer/Recevoir ultérieur.",
    ),
];

fn problems_of(app: &crate::MailApp, account: mailcraft_model::Id) -> Vec<String> {
    crate::sync_panel::rows(app).into_iter().find(|row| row.account == account).map(|row| row.problems).unwrap_or_default()
}

#[test]
fn the_status_bar_and_the_panel_show_a_failed_run_in_the_interface_s_language() {
    use crate::chrome::{SyncShown, sync_shown};
    let (mut app, account) = window();
    let english: Vec<String> = FAILED.iter().map(|(en, _)| (*en).to_string()).collect();
    let french: Vec<String> = FAILED.iter().map(|(_, fr)| (*fr).to_string()).collect();
    app.keep_sync_problems(account, &english);
    app.session.sync_status = english[0].clone();
    super::super::set_current(super::super::Lang::from_tag("fr").unwrap());
    // The bar says the first problem, the panel every one, and Copy problems copies them as they are shown.
    assert_eq!(sync_shown(&app), SyncShown::Text(french[0].clone()));
    assert_eq!(problems_of(&app, account), french);
    let copied = app.sync_problem_lines();
    assert!(french.iter().all(|line| copied.contains(&format!("  {line}"))), "{copied:#?}");
    // What is kept stays English: in English the same lines read as they came, and French again after.
    super::super::set_current(Lang::English);
    assert_eq!(sync_shown(&app), SyncShown::Text(english[0].clone()));
    assert_eq!(problems_of(&app, account), english);
    assert!(app.sync_problem_lines().iter().any(|l| l == &format!("  {}", english[0])));
    super::super::set_current(Lang::from_tag("fr").unwrap());
    assert_eq!(problems_of(&app, account), french);
    // How a run ended, with the time, and what the engine adds of the messages it can't find.
    for (status, shown) in [
        ("Up to date — 15:05", "À jour — 15:05"),
        ("Folders up to date — 3:05 PM", "Dossiers à jour — 3:05 PM"),
        (
            "Up to date. 2 moved messages can't be found on the server yet, so changes to them wait. — 15:05",
            "À jour. 2 messages déplacés n’ont pas encore été retrouvés sur le serveur : les modifications qui les concernent attendent. — 15:05",
        ),
        (
            "Couldn't reach the server: network: down. 1 moved message can't be found on the server yet, so changes to it wait. — 15:05",
            "Impossible de joindre le serveur : réseau : down. 1 message déplacé n’a pas encore été retrouvé sur le serveur : les modifications qui le concernent attendent. — 15:05",
        ),
        // What isn't a time after the dash is a server's words.
        ("Couldn't reach the server: network: closed — goodbye", "Impossible de joindre le serveur : réseau : closed — goodbye"),
        // A line no row reads, as it came.
        ("Receiving mail failed: the server closed the connection", "Receiving mail failed: the server closed the connection"),
    ] {
        app.session.sync_status = status.to_string();
        assert_eq!(sync_shown(&app), SyncShown::Text(shown.to_string()), "{status:?}");
    }
    super::super::set_current(Lang::English);
}

#[test]
fn the_panel_draws_the_lines_in_french() {
    let (mut app, account) = window();
    let english: Vec<String> = FAILED.iter().map(|(en, _)| (*en).to_string()).collect();
    app.keep_sync_problems(account, &english);
    app.sync_panel = crate::sync_panel::Panel::named("syncProblems");
    // (The window sets the language of its setting each frame.)
    app.ui.language = "fr".into();
    let harness = drawn(app);
    for (en, fr) in FAILED {
        assert!(harness.query_by_label(fr).is_some(), "{fr:?} isn't drawn");
        assert!(harness.query_by_label(en).is_none(), "{en:?} is drawn in English");
    }
    super::super::set_current(Lang::English);
}

#[test]
fn in_english_a_line_is_handed_back_as_it_came() {
    let mb = mailbox();
    let line = "Couldn't reach the server: network: connection refused";
    assert!(matches!(super::line(line, &mb), std::borrow::Cow::Borrowed(same) if std::ptr::eq(same, line)));
    assert!(matches!(super::status(line, &mb), std::borrow::Cow::Borrowed(same) if std::ptr::eq(same, line)));
    super::super::set_current(Lang::from_tag("fr").unwrap());
    // And in French, one no row reads.
    let other = "Receiving mail failed: the server closed the connection";
    assert!(matches!(super::line(other, &mb), std::borrow::Cow::Borrowed(same) if std::ptr::eq(same, other)));
    assert!(matches!(super::line(line, &mb), std::borrow::Cow::Owned(_)));
    super::super::set_current(Lang::English);
}

#[test]
fn what_sending_says_reads_in_french() {
    let mb = mailbox();
    let fr = |family: Family, text: &str| super::explain(french(), family, text, &mb).map(|r| r.text);
    assert_eq!(
        fr(Family::Line, "Couldn't send 'Hello': the server doesn't accept (address). It is held in the Outbox.").as_deref(),
        Some("Impossible d’envoyer « Hello » : le serveur n’accepte pas (adresse). Il est retenu dans la Boîte d’envoi.")
    );
    assert_eq!(
        fr(Family::Line, "'(no subject)' may have been sent: the server's answer was lost. It is held in the Outbox.").as_deref(),
        Some("« (sans objet) » a peut-être été envoyé : la réponse du serveur a été perdue. Il est retenu dans la Boîte d’envoi.")
    );
    assert_eq!(
        fr(Family::Banner, "It wasn't sent: the server doesn't accept bob@example.org. Change it, or send it again.").as_deref(),
        Some("Il n’a pas été envoyé : le serveur n’accepte pas bob@example.org. Modifiez-le, ou renvoyez-le.")
    );
    // A server's words stay as they came, under words of ours.
    assert_eq!(
        fr(Family::Banner, "It wasn't sent: the server refused it (somethingNew): It said so. Change it, or send it again.").as_deref(),
        Some("Il n’a pas été envoyé : le serveur l’a refusé (somethingNew) : It said so. Modifiez-le, ou renvoyez-le.")
    );
    // A subject that holds what follows it.
    assert_eq!(
        fr(Family::Line, "Couldn't send 'Re: plans': the details': the message has no recipients. It is held in the Outbox.").as_deref(),
        Some("Impossible d’envoyer « Re: plans': the details » : le message n’a aucun destinataire. Il est retenu dans la Boîte d’envoi.")
    );
}

/// The window, drawn for a few frames.
fn drawn(app: crate::MailApp) -> egui_kittest::Harness<'static, crate::MailApp> {
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut crate::MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

#[test]
fn the_banner_of_a_held_message_says_why_in_the_interface_s_language() {
    let (mut app, account) = window();
    let outbox = app.session.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap();
    let held = "It wasn't sent: the server doesn't accept bob@example.org. Change it, or send it again.";
    let message = mailcraft_model::Message { account, folder: outbox, subject: "Plans".into(), held: held.into(), ..Default::default() };
    let id = app.session.mailbox.add_message(message).unwrap();
    app.session.view.folder = outbox;
    app.ui.language = "fr".into();
    let mut harness = drawn(app);
    harness.state_mut().execute("mail.select", serde_json::json!({"id": id})).unwrap();
    harness.run_steps(3);
    let french = "Il n’a pas été envoyé : le serveur n’accepte pas bob@example.org. Modifiez-le, ou renvoyez-le.";
    assert!(harness.query_by_label(french).is_some(), "the banner in French");
    assert!(harness.query_by_label(held).is_none());
    // The mailbox keeps the English; in English it is shown as it is.
    assert_eq!(harness.state().session.mailbox.message(id).map(|m| m.held.clone()).as_deref(), Some(held));
    harness.state_mut().ui.language = "en".into();
    harness.run_steps(3);
    assert!(harness.query_by_label(held).is_some(), "the banner in English");
    super::super::set_current(Lang::English);
}

#[test]
fn a_refused_change_reads_in_french_with_its_words_agreed() {
    let mb = mailbox();
    let fr = |text: &str| super::explain(french(), Family::Line, text, &mb).map(|r| r.text);
    let cases = [
        (
            "Couldn't mark 'Invoice 42' as read: the server says it no longer exists.",
            "Impossible de marquer « Invoice 42 » comme lu : selon le serveur, il n’existe plus.",
        ),
        (
            "Couldn't mark 3 messages as unread: the server says they no longer exist.",
            "Impossible de marquer 3 messages comme non lus : selon le serveur, ils n’existent plus.",
        ),
        (
            "Couldn't change the flag of 2 messages: the server says this account may not change them.",
            "Impossible de modifier l’indicateur de 2 messages : selon le serveur, ce compte n’est pas autorisé à les modifier.",
        ),
        // A standard folder by the name the folder pane gives it.
        (
            "Couldn't move 'Invoice 42' to INBOX: the server says this account may not change it.",
            "Impossible de déplacer « Invoice 42 » vers le dossier Boîte de réception : selon le serveur, ce compte n’est pas autorisé à le modifier.",
        ),
        (
            "Couldn't delete 2 messages for good: the server doesn't allow it.",
            "Impossible de supprimer définitivement 2 messages : le serveur ne l’autorise pas.",
        ),
        (
            "Couldn't delete a message for good: the server says it can't make that change.",
            "Impossible de supprimer définitivement un message : selon le serveur, il ne peut pas faire cette modification.",
        ),
        // A subject that holds the words that follow it is split where the rest reads.
        (
            "Couldn't move 'a: the server says b' to Projects: the server says the mailbox is full.",
            "Impossible de déplacer « a: the server says b » vers le dossier Projects : selon le serveur, la boîte aux lettres est pleine.",
        ),
    ];
    for (english, french) in cases {
        assert_eq!(fr(english).as_deref(), Some(french), "{english:?}");
    }
}

#[test]
fn how_far_a_run_has_got_names_a_standard_folder_as_a_refused_move_does() {
    let mb = mailbox();
    for lang in [Lang::English, Lang::Pseudo, french()] {
        let named = super::standard_folder_name(&mb, "INBOX", lang);
        super::super::set_current(lang);
        assert_eq!(named.as_deref(), Some(crate::nav::folder_name(mb.folders.iter().find(|f| f.name == "INBOX").unwrap()).as_str()), "{lang:?}");
        assert_eq!(super::standard_folder_name(&mb, "Projects", lang), None);
    }
    super::super::set_current(Lang::English);
}

#[test]
fn a_refused_move_or_copy_says_why_in_french_in_the_status_and_the_box() {
    let (mut app, account) = window();
    let inbox = app.session.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let server = mailcraft_model::sync::ServerMessage { id: "M1".into(), folder: "Mi".into(), ..Default::default() };
    let message = mailcraft_model::Message { account, folder: inbox, subject: "Plans".into(), server: Some(server), ..Default::default() };
    let id = app.session.mailbox.add_message(message).unwrap();
    // (An account kept on this computer.)
    let mine =
        app.session.mailbox.add_account(mailcraft_model::Account { name: "Mine".into(), email: "mine@example.org".into(), ..Default::default() });
    let local = app.session.mailbox.folder_of_kind(mine, FolderKind::Inbox);
    let trash = app.session.mailbox.folder_of_kind(account, FolderKind::Deleted);
    super::super::set_current(french());
    app.by_hand = true;
    for (command, folder, french) in [
        (
            "mail.move",
            local,
            "Le courrier ne peut pas encore être déplacé entre un compte JMAP et un autre compte ou un dossier local : cette version ne peut pas placer un message sur un serveur JMAP, et la copie du serveur resterait où elle est",
        ),
        (
            "mail.copy",
            trash,
            "Le courrier ne peut être copié ni vers ni depuis un compte JMAP : son serveur garde un message dans un seul dossier. Déplacez-le, ou enregistrez-le sous forme de fichier",
        ),
    ] {
        assert!(app.run(command, serde_json::json!({"folder": folder, "ids": [id]})).is_err(), "{command}");
        assert_eq!(app.status_msg.as_ref().map(|(said, _)| said.as_str()), Some(french), "{command}: the status");
        assert_eq!(app.refused.take().as_deref(), Some(french), "{command}: the box");
    }
    // The engine's refusal of received mail for a Microsoft account reads as it did.
    assert_eq!(super::refusal(mailcraft_model::STAYS_LOCAL, &app.session.mailbox), t_in(french(), mailcraft_model::STAYS_LOCAL));
    super::super::set_current(Lang::English);
}

#[test]
fn what_undo_and_the_engine_refuse_reads_in_french() {
    let mb = mailbox();
    let fr = |text: &str| super::explain_status(french(), text, &mb).map(|r| r.text);
    let cases = [
        (
            "'Plans' was deleted for good: that can't be undone here.",
            "« Plans » a été supprimé définitivement : cette action ne peut pas être annulée ici.",
        ),
        ("Sending 2 messages can't be undone.", "L’envoi de 2 messages ne peut pas être annulé."),
        (
            "Ada's JMAP server can't be reached: is a plain http:// address to a server that isn't on this computer: use https://",
            "Le serveur JMAP du compte Ada est injoignable : est en http:// en clair pour un serveur qui n’est pas sur cet ordinateur : utilisez https://",
        ),
        (
            "A JMAP account's folders are its server's: this version can't make, rename, move or delete them there",
            "Les dossiers d’un compte JMAP sont ceux de son serveur : cette version ne peut pas les créer, les renommer, les déplacer ni les supprimer sur le serveur",
        ),
    ];
    for (english, french) in cases {
        assert_eq!(fr(english).as_deref(), Some(french), "{english:?}");
    }
}

#[test]
fn a_meeting_held_online_by_a_service_of_its_own_is_joined_in_french_as_one_online() {
    // (The card's menu: `Join {service} Meeting`, the service by its label.)
    let service = t_in(french(), mailcraft_model::OnlineProvider::Other.label());
    assert_eq!(fmt(t_in(french(), "Join {service} Meeting"), &[("service", service)]), "Rejoindre la réunion en ligne");
}

#[test]
fn a_notes_limit_reads_with_the_thousands_as_the_language_writes_them() {
    let mb = mailbox();
    let fr = |text: &str| super::explain_status(french(), text, &mb).map(|r| r.text);
    assert_eq!(fr("A note holds 100,000 characters at most").as_deref(), Some("Une note contient au plus 100'000 caractères"));
    assert_eq!(fr("A note's title holds 140 characters at most").as_deref(), Some("Le titre d’une note contient au plus 140 caractères"));
}
