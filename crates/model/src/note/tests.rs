// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Notes as the mailbox keeps them, as a file and a server hand them over, and repaired. Invented people at
//! `example.org`.

use proptest::prelude::*;
use serde_json::json;

use super::*;
use crate::Account;

fn round_trip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let json = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value, "{json}");
    json
}

/// A mailbox with a local account and a server's, and the server's account id.
fn mailbox() -> (Mailbox, Id) {
    let mut mailbox = Mailbox::with_local_account("Ada Marlow", "ada@example.org");
    let account = mailbox.add_account(Account { email: "ada@neoffice.example.org".into(), ..Default::default() });
    (mailbox, account)
}

fn kept(mailbox: &mut Mailbox, account: Id, remote: &str, title: &str) -> Id {
    let id = mailbox.alloc_id();
    let note = Note { id, origin: Origin { account, id: remote.into(), ..Default::default() }, title: title.into(), ..Default::default() };
    mailbox.notes.items.insert(id, Arc::new(note));
    id
}

#[test]
fn a_note_and_its_store_read_and_write_as_json_and_a_file_from_before_has_none() {
    // A mailbox from before notes: none, and nothing of them is written back.
    let (mut mailbox, account) = mailbox();
    let json = serde_json::to_string(&mailbox).unwrap();
    assert!(!json.contains("\"notes\""), "{json}");
    let read: Mailbox = serde_json::from_str(&json).unwrap();
    assert!(read.notes.is_empty() && read == mailbox);
    // Notes, a server, and a change: they come back as they were.
    let own = mailbox.alloc_id();
    mailbox
        .notes
        .items
        .insert(own, Arc::new(Note { id: own, title: "Groceries".into(), text: "Milk\nBread".into(), pinned: true, ..Default::default() }));
    let theirs = kept(&mut mailbox, account, "VN-2026-00012", "Room booking codes");
    if let Some(note) = mailbox.notes.note_mut(theirs) {
        (note.shared, note.owner_name, note.rights.write, note.version) = (true, "Grace Okafor".into(), false, "2026-10-20T08:00:00.123Z".into());
    }
    mailbox.notes.servers.push(NoteServer { account, writable: vec!["title".into(), "text".into()], ..Default::default() });
    mailbox.notes.changes.push(NoteChange {
        account,
        kind: NoteChangeKind::Update,
        note: theirs,
        remote: "VN-2026-00012".into(),
        ..Default::default()
    });
    let json = round_trip(&mailbox);
    assert!(json.contains("\"notes\":{") && json.contains("\"ownerName\":\"Grace Okafor\""), "{json}");
    // The mailbox's own keeps every right, which is not written.
    assert!(!json.contains("\"writeText\""), "{json}");
    // Something that isn't notes reads as none, rather than as a broken file.
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["notes"] = json!(5);
    assert!(serde_json::from_value::<Mailbox>(value.clone()).unwrap().notes.is_empty());
    value["notes"] = json!({"items": [{"id": 9, "title": "Kept"}, 5], "servers": [7, {"account": account}], "changes": "no"});
    let lenient: Mailbox = serde_json::from_value(value).unwrap();
    assert_eq!(lenient.notes.items.len(), 1);
    assert_eq!(lenient.notes.servers.len(), 1);
}

#[test]
fn a_remote_note_is_tidied_titles_one_line_text_capped_links_https_only() {
    let mut note = RemoteNote {
        id: "VN-2026-00012".into(),
        title: "Team\noffsite".into(),
        text: format!("Ideas\r\n\u{0}- boat\r{}", "x".repeat(MAX_NOTE_TEXT)),
        version: "2026-10-20T08:00:00.123Z".into(),
        web_url: "https://desk.example.org/app/notes?note=VN-2026-00012".into(),
        may_write: false,
        ..Default::default()
    };
    note.tidy();
    assert_eq!(note.title, "Team offsite");
    assert!(note.text.starts_with("Ideas\n- boat\n") && note.text.chars().count() == MAX_NOTE_TEXT && note.cut, "a text cut is said so");
    assert!(!note.may_write_text, "nothing of it is written where it may not be written");
    assert_eq!(note.web_url, "https://desk.example.org/app/notes?note=VN-2026-00012");
    for (link, kept) in [("http://desk.example.org/app/notes", ""), ("javascript:alert(1)", ""), ("https://bad host/x", "")] {
        let mut note = RemoteNote { id: "a".into(), web_url: link.into(), ..Default::default() };
        note.tidy();
        assert_eq!(note.web_url, kept, "{link}");
    }
    // A title longer than is kept is cut, and so is the note: its text is not changed here.
    let mut long = RemoteNote { id: "b".into(), title: "t".repeat(MAX_NOTE_TITLE + 1), version: "v".repeat(65), ..Default::default() };
    long.tidy();
    assert!(long.cut && long.title.chars().count() == MAX_NOTE_TITLE && long.version.is_empty());
    let here = long.note(3, 7);
    assert!(!here.text_writable() && here.rights.write, "its title may still change");
    // A download keeps one note per id, of those that have one, and as many as are kept of a server.
    let mut download = NotesDownload {
        complete: true,
        notes: vec![RemoteNote { id: "a".into(), ..Default::default() }, RemoteNote { id: "a".into(), ..Default::default() }, RemoteNote::default()],
        ..Default::default()
    };
    download.tidy();
    assert_eq!(download.notes.len(), 1);
    assert!(download.complete);
    download.notes = (0..MAX_SERVER_NOTES + 1).map(|n| RemoteNote { id: format!("n{n}"), ..Default::default() }).collect();
    download.tidy();
    assert!(download.notes.len() == MAX_SERVER_NOTES && !download.complete, "a list cut short is not whole");
}

#[test]
fn notes_and_their_changes_print_counts_only() {
    let (mut mailbox, account) = mailbox();
    let id = kept(&mut mailbox, account, "VN-1", "Salary review");
    if let Some(note) = mailbox.notes.note_mut(id) {
        note.text = "Confidential: 120k".into();
    }
    let fields = NoteFields { title: Some("Salary review".into()), text: Some("Confidential: 120k".into()), ..Default::default() };
    mailbox.notes.changes.push(NoteChange {
        account,
        kind: NoteChangeKind::Update,
        note: id,
        base: mailbox.notes.note(id).cloned().map(Box::new),
        ..Default::default()
    });
    let op = NoteOp { kind: NoteChangeKind::Update, note: id, fields: fields.clone(), ..Default::default() };
    let shown = [
        format!("{:?}", mailbox),
        format!("{:?}", mailbox.notes.note(id)),
        format!("{:?}", mailbox.notes.changes),
        format!("{op:?}"),
        format!("{:?}", NotesUpload { account, ops: vec![op.clone()], ..Default::default() }),
        format!("{:?}", RemoteNote { title: "Salary review".into(), text: "Confidential".into(), ..Default::default() }),
        format!(
            "{:?}",
            NotesAnswer {
                uploaded: Some(NotesUploaded { outcomes: vec![NoteOutcome { op, ..Default::default() }], ..Default::default() }),
                ..Default::default()
            }
        ),
    ];
    for line in shown {
        assert!(!line.contains("Salary") && !line.contains("Confidential"), "{line}");
    }
}

#[test]
fn repair_keeps_notes_of_accounts_here_once_each_with_ids_from_the_count() {
    let (mut mailbox, account) = mailbox();
    let first = kept(&mut mailbox, account, "VN-1", "First");
    let twin = kept(&mut mailbox, account, "VN-1", "Twin");
    let stray = kept(&mut mailbox, 999, "VN-2", "Of nobody here");
    // An id out of range, and one whose key is not its id.
    let big = (1u64 << 53) + 5;
    mailbox.notes.items.insert(big, Arc::new(Note { id: big, title: "Big".into(), ..Default::default() }));
    let own = mailbox.alloc_id();
    mailbox.notes.items.insert(own, Arc::new(Note { id: own + 100, title: "Mine\nsecond line".into(), shared: true, ..Default::default() }));
    mailbox.notes.servers = vec![
        NoteServer { account, ..Default::default() },
        NoteServer { account, ..Default::default() },
        NoteServer { account: 999, ..Default::default() },
    ];
    mailbox.notes.changes = vec![
        NoteChange { account, kind: NoteChangeKind::Update, note: first, remote: "VN-1".into(), ..Default::default() },
        NoteChange { account, kind: NoteChangeKind::Update, note: 4_242, remote: "VN-9".into(), ..Default::default() },
        NoteChange { account: 999, kind: NoteChangeKind::Delete, note: stray, remote: "VN-2".into(), ..Default::default() },
        NoteChange { account, kind: NoteChangeKind::Delete, note: 4_243, remote: "VN-3".into(), ..Default::default() },
    ];
    let fixed = mailbox.repair();
    assert!(!fixed.is_empty());
    let notes = &mailbox.notes;
    assert!(notes.note(first).is_some() && notes.note(twin).is_none() && notes.note(stray).is_none(), "{fixed:?}");
    assert!(notes.items.iter().all(|(key, note)| *key == note.id && *key < (1u64 << 53)));
    assert!(notes.items.values().any(|note| note.title == "Big"), "renumbered, not lost");
    let mine = notes.note(own).unwrap();
    assert!(mine.title == "Mine second line" && !mine.shared, "the mailbox's own is the person's");
    assert_eq!(notes.servers.len(), 1);
    let kinds: Vec<(NoteChangeKind, Id)> = notes.changes.iter().map(|change| (change.kind, change.note)).collect();
    assert_eq!(kinds, [(NoteChangeKind::Update, first), (NoteChangeKind::Delete, 4_243)]);
    // A note's id is never one an item already has.
    let mut ids: Vec<Id> = mailbox.messages.keys().chain(mailbox.contacts.keys()).chain(notes.items.keys()).copied().collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
    // An account that goes takes its notes along.
    mailbox.remove_account(account).unwrap();
    assert!(mailbox.notes.items.values().all(|note| note.origin.is_local()) && mailbox.notes.servers.is_empty() && mailbox.notes.changes.is_empty());
}

proptest! {
    #[test]
    fn hostile_notes_never_panic(text in any::<String>(), n in any::<u64>(), flags in any::<[bool; 4]>()) {
        let mut note = RemoteNote {
            id: text.clone(),
            title: text.clone(),
            text: text.repeat(3),
            version: text.clone(),
            web_url: text.clone(),
            owner_name: text.clone(),
            rich: flags[0],
            may_write: flags[1],
            ..Default::default()
        };
        note.tidy();
        prop_assert!(!note.title.contains('\n') && note.title.chars().count() <= MAX_NOTE_TITLE);
        prop_assert!(note.text.chars().all(|c| !c.is_control() || c == '\n' || c == '\t'));
        let here = note.note(n, n);
        prop_assert!(!here.text_writable() || (flags[1] && !flags[0]));
        let mut answer: NotesAnswer = serde_json::from_value(json!({
            "uploaded": {"outcomes": [{"op": {"remote": text, "fields": {"title": text}}, "error": text}], "oldState": text},
            "download": {"notes": [{"id": text, "text": text}], "server": {"maxTitle": n, "maxText": n, "writable": [text]}},
            "state": text,
        })).unwrap_or_default();
        answer.tidy();
        if let Some(download) = &answer.download {
            prop_assert!(download.server.max_title >= 1 && download.server.max_title <= MAX_NOTE_TITLE);
        }
        let (mut mailbox, account) = mailbox();
        let id = n % 1_000_000 + 1;
        mailbox.notes.items.insert(id, Arc::new(Note { id: n, origin: Origin { account: if flags[2] { account } else { n }, id: text.clone(), ..Default::default() }, title: text.clone(), ..Default::default() }));
        mailbox.notes.changes.push(NoteChange { account, note: id, kind: if flags[3] { NoteChangeKind::Delete } else { NoteChangeKind::Update }, remote: text.clone(), ..Default::default() });
        mailbox.repair();
        prop_assert!(mailbox.notes.items.iter().all(|(key, note)| *key == note.id));
    }
}

#[test]
fn the_state_of_the_notes_is_asked_about_while_it_is_less_than_an_hour_old() {
    let now = 1_791_993_600;
    let ask = |state: &str, ago: i64| NotesAsk { read: true, state: state.into(), read_at: now - ago, ..Default::default() };
    assert_eq!(ask("s1", 60).state_to_ask(now), Some("s1"));
    assert_eq!(ask("s1", NOTES_STATE_FOR - 1).state_to_ask(now), Some("s1"));
    for (state, ago) in [("s1", NOTES_STATE_FOR), ("s1", -60), ("", 60), ("s1", now)] {
        assert_eq!(ask(state, ago).state_to_ask(now), None, "{state:?} {ago}");
    }
}
