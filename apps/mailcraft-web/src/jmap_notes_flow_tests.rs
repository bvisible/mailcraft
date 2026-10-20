// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The whole way of a person's notes: the window's job (`jmap_notes.rs` of the window), the browser runner's core and
//! the in-process server, as the calendars' flows go ([`super::Flow`]). Nothing here leaves this machine; every name,
//! address and id is invented.

use mailcraft_engine::cmd::notes::{NO_NOTES_HERE, NOTE_NOT_YOURS, NOTE_READ_ONLY, NOTE_TEXT_ELSEWHERE};
use mailcraft_jmap::standin::{Effect, Notes, StoredNote};

use super::*;

/// A flow whose server keeps Ada's notes (`offered`): "Team offsite ideas" pinned, "Groceries", a rich "Quarterly
/// plan", "Room booking codes" Grace shares with her for reading, an archived "Old checklist"; in a hosting page's tab
/// that keeps nothing of its own when `hosted`. The first look has been taken.
fn start(hosted: bool, offered: bool) -> Flow {
    let server = StandIn::new();
    server.with(|d| {
        d.mailbox("Inbox", None, Some("inbox"));
        d.pim = Pim::offered();
        d.pim.own = vec![ME.into()];
        d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
        if offered {
            d.notes = Notes::offered();
            d.notes.add(StoredNote::new("VN-1", "Team offsite ideas", "Lake walk\nCooking class").pinned());
            d.notes.add(StoredNote::new("VN-2", "Groceries", "Milk\nBread"));
            d.notes.add(StoredNote::new("VN-3", "Quarterly plan", "Goals").rich());
            d.notes.add(StoredNote::new("VN-4", "Room booking codes", "4711").shared_by("grace@example.org", "Grace Okafor", false));
            d.notes.add(StoredNote::new("VN-5", "Old checklist", "Parking card").archived());
        }
    });
    let page = Rc::new(Page { token: RefCell::new(TOKEN.into()), server: server.clone(), renewals: RefCell::new(0) });
    let mut session = Session::in_zone(Mailbox::with_local_account("", ""), NOW, Zone::named("Europe/Zurich").unwrap());
    session.keeps_local = !hosted;
    let mut app = MailApp::new(session, Services::default());
    app.autosave = false;
    app.services.jmap = Some(Box::new(|_| mpsc::channel().1));
    app.services.jmap_host_credentials = true;
    let (to, credentials) = (server.clone(), page.clone());
    let (holding, held) = (Rc::new(Cell::new(false)), Rc::new(RefCell::new(None)));
    let (hold, keep) = (holding.clone(), held.clone());
    app.services.jmap_pim = Some(Box::new(move |job: JmapPimJob| {
        let (tx, rx) = mpsc::channel();
        if hold.get() {
            *keep.borrow_mut() = Some((job, tx));
        } else {
            let _ = tx.send(block_on(super::super::pim(to.clone(), &job, Some(credentials.as_ref()), |_| std::future::ready(()))));
        }
        rx
    }));
    let added = app.execute("account.add", json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    app.execute("view.calendar", json!({})).unwrap();
    let account = added["id"].as_u64().unwrap();
    let mut flow = Flow { app, server, page, account, at: 0.0, holding, held };
    assert!(flow.look(0.0), "the first look asks at once");
    flow
}

impl Flow {
    /// The titles of the notes Notes lists (the archived ones when `archived`), in its order.
    fn listed(&mut self, archived: bool) -> Vec<String> {
        let listed = self.app.execute("notes.list", json!({"archived": archived})).unwrap();
        listed["notes"].as_array().unwrap().iter().map(|note| note["title"].as_str().unwrap_or_default().to_string()).collect()
    }

    /// The note here whose heading is `title`.
    fn note(&self, title: &str) -> Option<mailcraft_model::note::Note> {
        self.app.session.mailbox.notes.items.values().find(|note| note.heading() == title).map(|note| note.as_ref().clone())
    }

    fn note_id(&self, title: &str) -> Id {
        self.note(title).map(|note| note.id).unwrap_or_else(|| panic!("no note {title}"))
    }

    /// The server's note of the title `title`.
    fn server_note(&self, title: &str) -> Option<StoredNote> {
        self.server.with(|d| d.notes.list.iter().find(|note| note.title == title).cloned())
    }

    /// What was changed of the notes goes: a look now, and one three seconds of quiet later. Whether a job went.
    fn push_notes(&mut self) -> bool {
        self.look(SECOND);
        self.look(4.0 * SECOND)
    }
}

#[test]
fn the_window_brings_the_persons_notes_and_the_module_shows_them() {
    let mut flow = start(false, true);
    assert_eq!(flow.listed(false), ["Team offsite ideas", "Room booking codes", "Quarterly plan", "Groceries"]);
    assert_eq!(flow.listed(true), ["Old checklist"]);
    flow.app.execute("view.notes", json!({})).unwrap();
    let codes = flow.note_id("Room booking codes");
    let got = flow.app.execute("notes.get", json!({"id": codes})).unwrap();
    assert_eq!((got["where"].as_str(), got["ownerName"].as_str(), got["rights"]["write"].as_bool()), (Some(ME), Some("Grace Okafor"), Some(false)));
    assert!(got["webUrl"].as_str().is_some_and(|url| url.starts_with("https://")));
    assert!(flow.app.jmap_notes_state(flow.account).is_some());
    // Notes in front: read again at once, then every two minutes; nothing changed, one question.
    let before = flow.server.calls().len();
    assert!(flow.look(SECOND));
    assert!(flow.server.calls().into_iter().skip(before).filter(|c| c.starts_with("Note/")).eq(["Note/query".to_string()]));
}

#[test]
fn a_note_is_made_edited_pinned_archived_and_deleted_on_the_server() {
    let mut flow = start(false, true);
    let lunch = flow.app.execute("notes.new", json!({"title": "Lunch", "text": "Friday"})).unwrap()["id"].as_u64().unwrap();
    assert!(flow.push_notes());
    let made = flow.server_note("Lunch").expect("made on the server");
    assert_eq!(made.text, "Friday");
    assert_eq!(flow.app.session.mailbox.notes.note(lunch).map(|note| note.origin.id.clone()), Some(made.id.clone()));
    let steps: [(&str, Value); 3] = [
        ("notes.edit", json!({"id": lunch, "text": "Friday at noon"})),
        ("notes.pin", json!({"id": lunch})),
        ("notes.archive", json!({"id": lunch})),
    ];
    for (command, params) in steps {
        flow.app.execute(command, params).unwrap();
        assert!(flow.push_notes(), "{command}");
    }
    let there = flow.server_note("Lunch").unwrap();
    assert!((there.text.as_str(), there.pinned, there.archived) == ("Friday at noon", true, true), "{there:?}");
    assert!(flow.app.session.mailbox.notes.changes.is_empty());
    flow.app.execute("notes.delete", json!({"id": lunch})).unwrap();
    assert!(flow.push_notes());
    assert!(flow.server_note("Lunch").is_none() && flow.note("Lunch").is_none());
    assert!(flow.app.sync_problems_of(flow.account).is_empty(), "{:?}", flow.app.sync_problems_of(flow.account));
}

#[test]
fn a_note_changed_elsewhere_meanwhile_keeps_both_versions() {
    let mut flow = start(false, true);
    let groceries = flow.note_id("Groceries");
    flow.app.execute("notes.edit", json!({"id": groceries, "text": "Milk\nBread\nEggs"})).unwrap();
    // Meanwhile, on the phone.
    flow.server.with(|d| d.notes.change("VN-2", |note| note.text = "Milk\nCoffee".into()));
    assert!(flow.push_notes());
    assert_eq!(flow.note("Groceries").map(|note| note.text), Some("Milk\nCoffee".into()), "the phone's, in the note");
    assert_eq!(flow.note("Groceries (2)").map(|note| note.text), Some("Milk\nBread\nEggs".into()), "Ada's, as a note of its own");
    let said = flow.app.sync_problems_of(flow.account);
    assert!(said.iter().any(|line| line.contains("kept as a note of its own")), "{said:?}");
    // The notes are read whole again soon (the state held was the server's no longer), and her copy goes too.
    let before = flow.server.calls().len();
    for _ in 0..3 {
        flow.push_notes();
    }
    assert!(flow.server.calls().into_iter().skip(before).any(|call| call == "Note/get"), "read again");
    assert_eq!(flow.server_note("Groceries (2)").map(|note| note.text), Some("Milk\nBread\nEggs".into()));
    assert_eq!(flow.server_note("Groceries").map(|note| note.text), Some("Milk\nCoffee".into()), "the phone's never written over");
}

#[test]
fn a_note_shared_for_reading_cannot_be_changed_and_says_why() {
    let mut flow = start(false, true);
    let codes = flow.note_id("Room booking codes");
    let refused = |flow: &mut Flow, command: &str, params: Value| flow.app.execute(command, params).unwrap_err();
    assert_eq!(refused(&mut flow, "notes.edit", json!({"id": codes, "text": "0000"})), NOTE_READ_ONLY);
    assert_eq!(refused(&mut flow, "notes.pin", json!({"id": codes})), NOTE_READ_ONLY);
    assert_eq!(refused(&mut flow, "notes.delete", json!({"id": codes})), NOTE_NOT_YOURS);
    // In the interface's language, the reason reads in French.
    flow.app.execute("ui.language", json!({"value": "fr"})).unwrap();
    let french = refused(&mut flow, "notes.delete", json!({"id": codes}));
    flow.app.execute("ui.language", json!({"value": "en"})).unwrap();
    assert!(french.contains("propriétaire") || french == NOTE_NOT_YOURS, "{french}");
    let before = flow.server.calls().len();
    flow.push_notes();
    assert!(!flow.server.calls().into_iter().skip(before).any(|c| c == "Note/set"), "nothing went");
    assert_eq!(flow.server_note("Room booking codes").map(|note| note.text), Some("4711".into()));
}

#[test]
fn a_rich_note_keeps_its_formatting_and_only_its_title_changes() {
    let mut flow = start(false, true);
    let plan = flow.note_id("Quarterly plan");
    let read = flow.note("Quarterly plan").map(|note| note.version).unwrap_or_default();
    assert_eq!(flow.app.execute("notes.edit", json!({"id": plan, "text": "Plain"})).unwrap_err(), NOTE_TEXT_ELSEWHERE);
    flow.app.execute("notes.edit", json!({"id": plan, "title": "Quarterly plan (draft)"})).unwrap();
    assert!(flow.push_notes());
    let there = flow.server_note("Quarterly plan (draft)").expect("renamed there");
    assert!(there.rich && there.text == "Goals", "{there:?}");
    let written = flow
        .server
        .with(|d| d.notes.written.iter().filter(|(what, _)| what.starts_with("update")).map(|(_, value)| value.clone()).collect::<Vec<_>>());
    // Its title alone, with the version it was read at: the server refuses a title over a newer one too.
    assert_eq!(written, [json!({"title": "Quarterly plan (draft)", "updated": read})]);
}

#[test]
fn a_token_that_dies_mid_job_sends_each_note_change_once() {
    let mut flow = start(false, true);
    let (groceries, offsite) = (flow.note_id("Groceries"), flow.note_id("Team offsite ideas"));
    flow.app.execute("notes.edit", json!({"id": groceries, "title": "Shopping"})).unwrap();
    flow.app.execute("notes.pin", json!({"id": offsite})).unwrap();
    // One note a call; the session ends once the first went and before the second could: the page hands a new token.
    flow.server.with(|d| {
        d.max_set = 1;
        d.faults.push_back(Fault::later("Note/set", 1, Effect::RefuseWith(401)));
    });
    assert!(flow.push_notes());
    assert_eq!(*flow.page.renewals.borrow(), 1);
    let sent: Vec<String> = flow.server.with(|d| d.notes.written.iter().map(|(what, _)| what.clone()).collect());
    assert_eq!(sent.len(), 2, "each once: {sent:?}");
    assert!(sent.contains(&"update VN-2".to_string()) && sent.contains(&"update VN-1".to_string()));
    assert!(flow.app.session.mailbox.notes.changes.is_empty(), "and both answers were kept");
    assert_eq!(flow.server_note("Shopping").map(|note| note.id), Some("VN-2".into()));
    assert!(flow.server_note("Team offsite ideas").is_some_and(|note| !note.pinned));
    assert!(!flow.app.jmap_refused_for(flow.account));
    // The first refused as stale (the desk renamed it meanwhile), then the token dies: the retry sends the second
    // alone, and reads the notes whole, so that the desk's title shows here at once and hers is a note of its own.
    let mut flow = start(false, true);
    let (groceries, offsite) = (flow.note_id("Groceries"), flow.note_id("Team offsite ideas"));
    flow.app.execute("notes.edit", json!({"id": groceries, "title": "Shopping"})).unwrap();
    flow.app.execute("notes.pin", json!({"id": offsite})).unwrap();
    flow.server.with(|d| {
        d.notes.change("VN-2", |note| note.title = "Market".into());
        d.max_set = 1;
        d.faults.push_back(Fault::later("Note/set", 1, Effect::RefuseWith(401)));
    });
    assert!(flow.push_notes());
    assert_eq!(*flow.page.renewals.borrow(), 1);
    let sent: Vec<String> = flow.server.with(|d| d.notes.written.iter().map(|(what, _)| what.clone()).collect());
    assert_eq!(sent.iter().filter(|what| *what == "update VN-2").count(), 1, "{sent:?}");
    assert_eq!(sent.iter().filter(|what| *what == "update VN-1").count(), 1, "{sent:?}");
    assert_eq!(flow.note("Market").map(|note| note.id), Some(groceries), "the desk's title, here at once");
    assert!(flow.note("Shopping (2)").is_some(), "hers, as a note of its own");
    assert_eq!(flow.server_note("Market").map(|note| note.id), Some("VN-2".into()), "never written over");
}

#[test]
fn a_hosting_tab_whose_server_offers_no_notes_shows_no_notes_module() {
    let mut flow = start(true, false);
    for (command, params) in [("view.notes", json!({})), ("notes.new", json!({"title": "Lost"}))] {
        let why = flow.app.execute(command, params).unwrap_err();
        assert!(why.ends_with(NO_NOTES_HERE), "{command}: {why}");
    }
    assert!(flow.app.session.mailbox.notes.items.is_empty());
    // The server keeps notes from now on: the next wide job brings them, and Notes is there.
    flow.server.with(|d| {
        d.notes = Notes::offered();
        d.notes.add(StoredNote::new("VN-1", "Team offsite ideas", "Lake walk"));
    });
    for _ in 0..5 {
        flow.look(2.0 * MINUTE + SECOND);
    }
    flow.app.execute("view.notes", json!({})).unwrap();
    assert_eq!(flow.listed(false), ["Team offsite ideas"]);
}

#[test]
fn a_note_whose_answer_was_lost_is_one_note() {
    let mut flow = start(false, true);
    let lunch = flow.app.execute("notes.new", json!({"title": "Lunch"})).unwrap()["id"].as_u64().unwrap();
    flow.server.with(|d| d.lost_answers = 1);
    assert!(flow.push_notes());
    assert!(flow.server_note("Lunch").is_some(), "made there all the same");
    // A while later it goes again with its mark: the same note there, and one here.
    flow.app.session.clock.now = Some(NOW + 5 * 60);
    for _ in 0..3 {
        flow.look(MINUTE);
    }
    assert_eq!(flow.server.with(|d| d.notes.list.iter().filter(|note| note.title == "Lunch").count()), 1);
    let here: Vec<Id> = flow.app.session.mailbox.notes.items.values().filter(|note| note.heading() == "Lunch").map(|note| note.id).collect();
    assert_eq!(here, [lunch], "the one made here, with the server's name");
    assert!(flow.app.session.mailbox.notes.changes.is_empty());
}
