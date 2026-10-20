// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The tests of `run_words`: the matcher over a table of its own, then the table of the crates' sentences.

use std::time::{Duration, Instant};

use mailcraft_model::{FolderKind, Mailbox};

use super::super::fmt;
use super::pattern::{Budget, Part, Pattern};
use super::{Family, Lang, Row, Table, read_steps, read_with};

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
