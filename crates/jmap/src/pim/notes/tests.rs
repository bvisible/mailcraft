// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A job's notes against the in-process server: the changes written first and what came of each, then the notes
//! read, the state that spares a whole read, and what trouble comes to. Invented people at `example.org`.

use std::sync::atomic::AtomicBool;

use mailcraft_model::note::{NoteFields, NotesAsk};
use mailcraft_model::sync::JmapPimRequest;

use super::*;
use crate::pim::sync_pim;
use crate::standin::{Effect, Fault, Notes, SESSION, StandIn, StoredNote, TOKEN};
use crate::{Auth, block_on};

const ME: &str = "ada@example.org";
/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A server with Ada's notes: "Offsite" pinned, "Groceries", a rich "Plan", "Codes" Grace shares for reading,
/// "Shared plan" Bob shares for writing, an archived "Old".
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let notes = &mut d.notes;
        *notes = Notes::offered();
        notes.add(StoredNote::new("VN-1", "Offsite", "Lake walk").pinned());
        notes.add(StoredNote::new("VN-2", "Groceries", "Milk\nBread"));
        notes.add(StoredNote::new("VN-3", "Plan", "Goals").rich());
        notes.add(StoredNote::new("VN-4", "Codes", "4711").shared_by("grace@example.org", "Grace Okafor", false));
        notes.add(StoredNote::new("VN-5", "Shared plan", "Steps").shared_by("bob@example.org", "Bob Lindqvist", true));
        notes.add(StoredNote::new("VN-6", "Old", "Done").archived());
    });
    server
}

/// A job that asks for the notes alone, as `ask` says.
fn request(ask: NotesAsk) -> JmapPimRequest {
    JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        now: NOW,
        notes: Some(ask),
        ..Default::default()
    }
}

fn read() -> NotesAsk {
    NotesAsk { read: true, ..Default::default() }
}

/// Reading, holding `state` from a whole read `ago` seconds before.
fn holding(state: &str, ago: i64) -> NotesAsk {
    NotesAsk { read: true, state: state.into(), read_at: NOW - ago, ..Default::default() }
}

/// Reading after writing `ops`.
fn writing(ops: Vec<NoteOp>) -> NotesAsk {
    NotesAsk { read: true, changes: Some(NotesUpload { account: 7, email: ME.into(), ops }), ..Default::default() }
}

fn run(server: &StandIn, request: &JmapPimRequest) -> JmapPimResult {
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    block_on(sync_pim(&client, request, &AtomicBool::new(false)))
}

fn answer(result: &JmapPimResult) -> &NotesAnswer {
    result.notes.as_ref().unwrap_or_else(|| panic!("no notes: {result:?}"))
}

fn download(result: &JmapPimResult) -> &NotesDownload {
    answer(result).download.as_ref().unwrap_or_else(|| panic!("no download: {result:?}"))
}

fn ids(result: &JmapPimResult) -> Vec<String> {
    download(result).notes.iter().map(|note| note.id.clone()).collect()
}

fn outcomes(result: &JmapPimResult) -> Vec<(NoteResult, String)> {
    answer(result).uploaded.as_ref().map_or_else(Vec::new, |uploaded| uploaded.outcomes.iter().map(|o| (o.result, o.error.clone())).collect())
}

fn create(note: u64, title: &str, token: &str) -> NoteOp {
    NoteOp {
        kind: NoteChangeKind::Create,
        note,
        token: token.into(),
        fields: NoteFields { title: Some(title.into()), text: Some("Words".into()), pinned: Some(false), archived: Some(false) },
        ..Default::default()
    }
}

fn update(remote: &str, fields: NoteFields, version: &str) -> NoteOp {
    NoteOp { kind: NoteChangeKind::Update, note: 1, remote: remote.into(), fields, version: version.into(), ..Default::default() }
}

fn destroy(remote: &str) -> NoteOp {
    NoteOp { kind: NoteChangeKind::Delete, note: 1, remote: remote.into(), ..Default::default() }
}

fn version_of(server: &StandIn, id: &str) -> String {
    server.with(|d| d.notes.note(id).map(|note| crate::standin::notes_version(note.updated)).unwrap_or_default())
}

#[test]
fn the_notes_are_read_whole_page_after_page_with_their_rights_and_links() {
    let server = server();
    server.with(|d| d.notes.limit = 2);
    let result = run(&server, &request(read()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let got = download(&result);
    assert!(got.offered && got.complete);
    assert_eq!(ids(&result), ["VN-1", "VN-6", "VN-5", "VN-4", "VN-3", "VN-2"], "archived ones too, in the server's order");
    assert_eq!((got.server.max_title, got.server.max_text, got.server.writable.len()), (140, 100_000, 4));
    let codes = got.notes.iter().find(|note| note.id == "VN-4").unwrap();
    assert!(codes.shared && !codes.may_write && !codes.may_write_text && !codes.may_delete);
    assert_eq!((codes.owner_name.as_str(), codes.web_url.as_str()), ("Grace Okafor", "https://jmap.standin.example/app/notes?note=VN-4"));
    assert!(codes.version.ends_with('Z') && codes.updated > 0 && codes.created > 0);
    let plan = got.notes.iter().find(|note| note.id == "VN-3").unwrap();
    assert!(plan.rich && plan.may_write && !plan.may_write_text);
    assert!(got.notes.iter().find(|note| note.id == "VN-6").is_some_and(|note| note.archived));
    // Three pages of the query, then one get.
    let calls = server.calls();
    assert_eq!(calls.iter().filter(|c| *c == "Note/query").count(), 3, "{calls:?}");
    assert_eq!(calls.iter().filter(|c| *c == "Note/get").count(), 1, "{calls:?}");
    assert_eq!(answer(&result).state, Some(server.with(|d| d.notes.state())));
}

#[test]
fn an_unchanged_state_reads_one_id_and_nothing_else() {
    let server = server();
    let state = answer(&run(&server, &request(read()))).state.clone().unwrap();
    let before = server.calls().len();
    let result = run(&server, &request(holding(&state, 600)));
    assert!(answer(&result).download.is_none());
    assert_eq!(answer(&result).state.as_deref(), Some(state.as_str()));
    let asked = server.asked();
    let asked: Vec<_> = asked.iter().skip(before).collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!((asked[0].method.as_str(), asked[0].limit), ("Note/query", Some(1)));
    // Changed elsewhere since: all of it is read.
    server.with(|d| d.notes.change("VN-2", |note| note.text = "Milk".into()));
    let result = run(&server, &request(holding(&state, 600)));
    assert_eq!(download(&result).notes.len(), 6);
    assert_ne!(answer(&result).state.as_deref(), Some(state.as_str()));
}

#[test]
fn a_state_older_than_an_hour_is_read_through() {
    let server = server();
    let state = answer(&run(&server, &request(read()))).state.clone().unwrap();
    let result = run(&server, &request(holding(&state, 3_600)));
    assert!(download(&result).complete && download(&result).notes.len() == 6);
    // A state from after now is no state either.
    let result = run(&server, &request(holding(&state, -60)));
    assert!(answer(&result).download.is_some());
}

#[test]
fn a_get_that_fails_is_halved_and_the_note_that_breaks_it_left_out() {
    let server = server();
    server.with(|d| d.notes.broken.insert("VN-3".into()));
    let result = run(&server, &request(read()));
    assert_eq!(ids(&result), ["VN-1", "VN-6", "VN-5", "VN-4", "VN-2"]);
    assert!(!download(&result).complete && answer(&result).state.is_none());
    assert_eq!(result.errors.len(), 1, "{:?}", result.errors);
    assert!(result.errors[0].starts_with("1 note couldn't be read: "), "{:?}", result.errors);
}

#[test]
fn text_beyond_the_budget_ends_the_read_incomplete() {
    let server = StandIn::new();
    let words = "w".repeat(100_000);
    server.with(|d| {
        d.notes = Notes::offered();
        for n in 0..90 {
            d.notes.add(StoredNote::new(&format!("VN-{n}"), &format!("N{n}"), &words));
        }
    });
    let result = run(&server, &request(read()));
    let got = download(&result);
    assert!(!got.complete && answer(&result).state.is_none());
    let read: usize = got.notes.iter().map(|note| note.text.chars().count() + note.title.chars().count()).sum();
    assert!(read <= TEXT_BUDGET && got.notes.len() >= 75, "{} notes, {read} characters", got.notes.len());
    assert_eq!(result.errors.len(), 1);
    assert!(
        result.errors[0].ends_with("notes couldn't be read: the notes hold more text than one reading takes, and these are not shown"),
        "{:?}",
        result.errors
    );
}

#[test]
fn a_note_longer_than_kept_comes_cut_and_read_only() {
    let server = server();
    server.with(|d| d.notes.add(StoredNote::new("VN-9", "Long", &"x".repeat(150_000))));
    let mut result = run(&server, &request(read()));
    result.tidy();
    let long = download(&result).notes.iter().find(|note| note.id == "VN-9").cloned().unwrap();
    assert!(long.cut && long.text.chars().count() <= mailcraft_model::note::MAX_NOTE_TEXT);
    assert!(!long.note(7, 1).rights.write_text && long.note(7, 1).rights.write, "its title may still change");
}

#[test]
fn changes_go_first_creates_with_their_uid_updates_as_whole_properties_and_the_version_for_words_only() {
    let server = server();
    let groceries = version_of(&server, "VN-2");
    let ops = vec![
        create(10, "Lunch", "mark-10"),
        update("VN-2", NoteFields { text: Some("Milk".into()), ..Default::default() }, &groceries),
        update("VN-1", NoteFields { pinned: Some(false), ..Default::default() }, ""),
        destroy("VN-6"),
    ];
    let before = server.calls().len();
    let result = run(&server, &request(writing(ops)));
    assert_eq!(outcomes(&result).iter().map(|(result, _)| *result).collect::<Vec<_>>(), [NoteResult::Done; 4], "{result:?}");
    let calls: Vec<String> = server.calls().into_iter().skip(before).collect();
    assert_eq!(calls.first().map(String::as_str), Some("Note/set"), "the changes go first: {calls:?}");
    assert!(calls.iter().skip(1).all(|c| c != "Note/set"));
    let written = server.with(|d| d.notes.written.clone());
    let sent = |what: &str| written.iter().find(|(said, _)| said == what).map(|(_, value)| value.clone());
    assert_eq!(sent("create"), Some(json!({"uid": "mark-10", "title": "Lunch", "text": "Words", "isPinned": false, "isArchived": false})));
    assert_eq!(sent("update VN-2"), Some(json!({"text": "Milk", "updated": groceries})));
    assert_eq!(sent("update VN-1"), Some(json!({"isPinned": false})), "a pin alone carries no version");
    assert_eq!((sent("destroy VN-6"), written.len()), (Some(Value::Null), 4));
    // What was made comes back whole; a change, with its new version.
    let uploaded = answer(&result).uploaded.as_ref().unwrap();
    let made = uploaded.outcomes[0].note.as_ref().unwrap();
    assert!(made.id.starts_with("VN-") && made.title == "Lunch" && made.web_url.starts_with("https://"));
    let changed = uploaded.outcomes[1].note.as_ref().unwrap();
    assert_eq!(changed.version, version_of(&server, "VN-2"));
    // And what is read after shows them.
    assert!(ids(&result).contains(&made.id) && !ids(&result).contains(&"VN-6".to_string()));
    assert!(!uploaded.old_state.is_empty() && !uploaded.new_state.is_empty() && uploaded.old_state != uploaded.new_state);
}

#[test]
fn outcomes_done_gone_stale_refused_failed_follow_the_set_errors() {
    let server = server();
    let groceries = version_of(&server, "VN-2");
    server.with(|d| {
        d.notes.change("VN-2", |note| note.text = "Changed on the phone".into());
        d.notes.refused.insert("VN-1".into(), "serverFail");
        d.notes.refused.insert("mark-full".into(), "overQuota");
        d.notes.max_title = 10;
    });
    let ops = vec![
        update("VN-2", NoteFields { text: Some("Mine".into()), ..Default::default() }, &groceries),
        update("VN-404", NoteFields { title: Some("x".into()), ..Default::default() }, "2026-01-01T00:00:00Z"),
        update("VN-4", NoteFields { title: Some("Mine now".into()), ..Default::default() }, &version_of(&server, "VN-4")),
        destroy("VN-5"),
        update("VN-1", NoteFields { pinned: Some(false), ..Default::default() }, ""),
        create(11, "Too many", "mark-full"),
        create(12, "A title too long", "mark-12"),
        update("VN-3", NoteFields { text: Some("Plain".into()), ..Default::default() }, &version_of(&server, "VN-3")),
    ];
    let result = run(&server, &request(writing(ops)));
    let said = outcomes(&result);
    let kinds: Vec<NoteResult> = said.iter().map(|(result, _)| *result).collect();
    use NoteResult::{Failed, Gone, Refused, Stale};
    assert_eq!(kinds, [Stale, Gone, Refused, Refused, Failed, Failed, Refused, Refused], "{said:?}");
    assert_eq!(said[3].1, "Only its owner deletes a note.");
    assert!(said[6].1.starts_with("A title is one line"), "{said:?}");
    let uploaded = answer(&result).uploaded.as_ref().unwrap();
    assert!(uploaded.outcomes.iter().filter(|o| o.result == Failed).all(|o| o.retry_after == Some(60)));
    // Something the server didn't take as sent: the notes are read whole again, and no state is vouched for but theirs.
    assert!(download(&result).complete);
}

#[test]
fn a_create_sent_again_after_a_lost_answer_is_the_same_note() {
    let server = server();
    server.with(|d| d.lost_answers = 1);
    let op = create(13, "Lunch", "mark-13");
    let first =
        run(&server, &request(NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![op.clone()] }), ..Default::default() }));
    assert_eq!(outcomes(&first).first().map(|(result, _)| *result), Some(NoteResult::Failed), "{first:?}");
    let again = run(&server, &request(writing(vec![op])));
    let made = answer(&again).uploaded.as_ref().and_then(|uploaded| uploaded.outcomes.first()).cloned().unwrap();
    assert_eq!(made.result, NoteResult::Done);
    assert_eq!(server.with(|d| d.notes.list.iter().filter(|note| note.title == "Lunch").count()), 1);
    assert_eq!(ids(&again).iter().filter(|id| Some(id.as_str()) == made.note.as_ref().map(|note| note.id.as_str())).count(), 1);
}

#[test]
fn the_state_after_a_set_is_the_new_state_when_the_old_one_was_held() {
    let server = server();
    let state = answer(&run(&server, &request(read()))).state.clone().unwrap();
    let before = server.calls().len();
    let pin = update("VN-2", NoteFields { pinned: Some(true), ..Default::default() }, "");
    let result = run(
        &server,
        &request(NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![pin.clone()] }), ..holding(&state, 60) }),
    );
    assert!(answer(&result).download.is_none(), "nothing changed but what was sent");
    let now = server.with(|d| d.notes.state());
    assert_eq!(answer(&result).state.as_deref(), Some(now.as_str()));
    assert_eq!(server.calls().into_iter().skip(before).collect::<Vec<_>>(), ["Note/set", "Note/query"]);
    // Changed elsewhere before the set: the state held is no longer the server's, and everything is read.
    server.with(|d| d.notes.change("VN-1", |note| note.text = "On the phone".into()));
    let unpin = update("VN-2", NoteFields { pinned: Some(false), ..Default::default() }, "");
    let result =
        run(&server, &request(NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![unpin] }), ..holding(&now, 60) }));
    assert!(download(&result).complete);
    assert_eq!(answer(&result).state, Some(server.with(|d| d.notes.state())));
}

#[test]
fn a_session_without_the_capability_reads_no_notes_and_says_so() {
    let server = server();
    server.with(|d| d.notes.offered = false);
    let result = run(&server, &request(read()));
    let gone = download(&result);
    assert!(!gone.offered && gone.notes.is_empty() && result.errors.is_empty());
    assert_eq!(server.api_requests(), 0, "nothing asked of the server");
    // A job that asks nothing of the notes is told nothing of them.
    let result = run(&server, &JmapPimRequest { notes: None, ..request(read()) });
    assert!(result.notes.is_none());
}

#[test]
fn trouble_ends_the_notes_as_it_ends_the_contacts() {
    let pin = || writing(vec![update("VN-2", NoteFields { pinned: Some(true), ..Default::default() }, "")]);
    // The credentials refused while writing: the change has no outcome and waits, and nothing is read.
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.drop_token_after = Some(0));
    let result = block_on(sync_pim(&client, &request(pin()), &AtomicBool::new(false)));
    assert!(result.auth_refused, "{result:?}");
    let uploaded = answer(&result).uploaded.as_ref().unwrap();
    assert!(uploaded.token_refused && uploaded.outcomes.is_empty());
    assert!(answer(&result).download.is_none() && answer(&result).state.is_none());
    // A busy server, for the time it says: the change waits that long.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.refuse_next.push_back(Some((429, Some(90)))));
    let result = block_on(sync_pim(&client, &request(pin()), &AtomicBool::new(false)));
    assert_eq!(result.busy_for, Some(90));
    assert_eq!(answer(&result).uploaded.as_ref().unwrap().outcomes[0].retry_after, Some(90));
    assert!(answer(&result).download.is_none());
    // Busy while reading.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    server.with(|d| d.refuse_next.push_back(Some((503, None))));
    let result = block_on(sync_pim(&client, &request(read()), &AtomicBool::new(false)));
    assert_eq!(result.busy_for, Some(30));
    assert!(answer(&result).state.is_none());
    // Stopped: nothing is asked.
    let server = self::server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    let result = block_on(sync_pim(&client, &request(pin()), &AtomicBool::new(true)));
    assert!(result.stopped && server.api_requests() == 0);
    // A query the server fails: a line, and no state.
    let server = self::server();
    server.with(|d| d.faults.push_back(Fault::on("Note/query", Effect::MethodError("serverFail"))));
    let result = run(&server, &request(read()));
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].starts_with("Couldn't read the notes: "), "{:?}", result.errors);
    assert!(answer(&result).state.is_none() && answer(&result).download.is_none());
}

#[test]
fn the_lines_name_no_note() {
    let server = server();
    server.with(|d| {
        d.notes.broken.insert("VN-1".into());
        d.faults.push_back(Fault::on("Note/set", Effect::MethodError("serverFail")));
    });
    let ops = vec![
        create(10, "Secret plans of Ada", "mark-10"),
        update("VN-2", NoteFields { title: Some("Groceries for Grace".into()), ..Default::default() }, ""),
    ];
    let result = run(&server, &JmapPimRequest { session: format!("{SESSION}?account={ME}"), ..request(writing(ops)) });
    assert_eq!(result.errors.len(), 2, "{:?}", result.errors);
    assert!(result.errors[0].starts_with("Couldn't send the note changes: "));
    for line in &result.errors {
        for word in ["Secret", "Groceries", "Offsite", "VN-", "@", "example.org", "Grace", "Ada"] {
            assert!(!line.contains(word), "{word} in {line}");
        }
    }
}

#[test]
fn a_server_that_takes_one_call_a_request_is_asked_in_turn() {
    let server = server();
    server.with(|d| {
        d.max_calls = 1;
        d.max_get = 2;
    });
    let result = run(&server, &request(writing(vec![update("VN-2", NoteFields { pinned: Some(true), ..Default::default() }, "")])));
    assert!(result.errors.is_empty() && download(&result).complete, "{:?}", result.errors);
    assert_eq!(server.calls().iter().filter(|c| *c == "Note/get").count(), 3, "six notes, two a get");
}

#[test]
fn the_session_names_the_notes_account_and_its_limits_clamped() {
    let server = server();
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert!(client.offers_notes());
    assert_eq!(client.notes_account(), Some(crate::standin::ACCOUNT));
    assert_eq!(client.notes_limits(), (140, 100_000));
    assert_eq!(client.notes_writable().map(<[String]>::len), Some(4));
    for (said, limits) in [
        (json!({"maxSizeTitle": 5_000, "maxSizeText": 1_000_000}), (1_000, 100_000)),
        (json!({"maxSizeTitle": 0, "maxSizeText": "long"}), (1, 100_000)),
        (json!({}), (140, 100_000)),
        (json!("nothing"), (140, 100_000)),
    ] {
        assert_eq!(limits_of(Some(&said)), limits, "{said}");
    }
    assert_eq!(limits_of(None), (140, 100_000));
    // A session without the capability names no account.
    server.with(|d| d.notes.offered = false);
    let client = block_on(Client::connect(server.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert!(!client.offers_notes() && client.notes_account().is_none());
}

fn limits_of(capability: Option<&Value>) -> (usize, usize) {
    limits(capability)
}

#[test]
fn a_job_that_only_writes_reads_when_a_change_was_not_taken_as_sent() {
    let server = server();
    let pin = update("VN-2", NoteFields { pinned: Some(true), ..Default::default() }, "");
    let only = |ops: Vec<NoteOp>| NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops }), ..Default::default() };
    // Taken: nothing is read.
    let result = run(&server, &request(only(vec![pin])));
    assert!(answer(&result).download.is_none(), "{result:?}");
    // Stale: what the server has now comes with the answer.
    let old = version_of(&server, "VN-1");
    server.with(|d| d.notes.change("VN-1", |note| note.text = "On the phone".into()));
    let result = run(&server, &request(only(vec![update("VN-1", NoteFields { text: Some("Mine".into()), ..Default::default() }, &old)])));
    assert_eq!(outcomes(&result)[0].0, NoteResult::Stale);
    let read = download(&result);
    assert!(read.complete && read.notes.iter().any(|note| note.id == "VN-1" && note.text == "On the phone"));
}

#[test]
fn a_change_of_words_without_a_version_is_not_sent_and_the_notes_are_read() {
    let server = server();
    let words = update("VN-2", NoteFields { text: Some("Mine".into()), ..Default::default() }, "");
    let ask = NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![words] }), ..Default::default() };
    let result = run(&server, &request(ask));
    assert_eq!(outcomes(&result).first().map(|(result, _)| *result), Some(NoteResult::Failed), "{result:?}");
    assert!(!server.with(|d| d.notes.written.iter().any(|(what, _)| what == "update VN-2")), "never written over whatever is there");
    assert!(answer(&result).download.as_ref().is_some_and(|read| read.complete), "read, for the version the words are made from");
    assert_eq!(server.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk\nBread".into()));
}

#[test]
fn a_change_whose_answer_was_lost_is_no_conflict_with_itself() {
    let server = server();
    let read = version_of(&server, "VN-2");
    let change = update("VN-2", NoteFields { text: Some("Milk".into()), ..Default::default() }, &read);
    let only = |op: NoteOp| NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![op] }), ..Default::default() };
    // The server writes it, and its answer is lost on the way.
    server.with(|d| d.lost_answers = 1);
    let first = run(&server, &request(only(change.clone())));
    assert_eq!(outcomes(&first)[0].0, NoteResult::Failed);
    assert_eq!(server.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk".into()));
    // Sent again, from the version it was made from: the note there has those very words, and it is done.
    let again = run(&server, &request(only(change)));
    let said = answer(&again).uploaded.as_ref().and_then(|uploaded| uploaded.outcomes.first()).cloned().unwrap();
    assert_eq!(said.result, NoteResult::Done, "{said:?}");
    assert_eq!(said.note.map(|note| (note.text, note.version)), Some(("Milk".into(), version_of(&server, "VN-2"))));
}

#[test]
fn a_note_that_says_nothing_of_the_rights_allows_a_shared_one_nothing() {
    let shared = note_of(&json!({"id": "VN-1", "title": "Codes", "isShared": true})).unwrap();
    assert!(!shared.may_write && !shared.may_write_text && !shared.may_delete);
    let own = note_of(&json!({"id": "VN-2", "title": "Mine"})).unwrap();
    assert!(own.may_write && own.may_write_text && own.may_delete);
    // An answer to a change that doesn't say the note's words says nothing of it.
    assert!(changed_of("VN-2", &json!({"updated": "2026-10-20T08:00:00.123Z"})).is_none());
    assert!(changed_of("VN-2", &json!({"updated": "2026-10-20T08:00:00.123Z", "title": "T", "text": ""})).is_some());
}

#[test]
fn a_change_without_a_version_has_the_notes_read_whole_even_with_a_state_held() {
    let server = server();
    let state = answer(&run(&server, &request(read()))).state.clone().unwrap();
    let words = update("VN-2", NoteFields { text: Some("Mine".into()), ..Default::default() }, "");
    let ask = NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![words] }), ..holding(&state, 60) };
    let result = run(&server, &request(ask));
    assert!(answer(&result).download.as_ref().is_some_and(|read| read.complete), "read whole, for the version: {result:?}");
}

#[test]
fn more_typed_after_a_lost_answer_goes_over_the_servers_copy_of_her_own_write() {
    let server = server();
    let read = version_of(&server, "VN-2");
    let only = |op: NoteOp| NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![op] }), ..Default::default() };
    // "Milk" goes; the server writes it and its answer is lost.
    server.with(|d| d.lost_answers = 1);
    run(&server, &request(only(update("VN-2", NoteFields { text: Some("Milk".into()), ..Default::default() }, &read))));
    // She types more meanwhile: the change now carries "Milk\nEggs", and what went before.
    let more = NoteOp {
        earlier: vec![NoteFields { text: Some("Milk".into()), ..Default::default() }],
        ..update("VN-2", NoteFields { text: Some("Milk\nEggs".into()), ..Default::default() }, &read)
    };
    let again = run(&server, &request(only(more)));
    let said = answer(&again).uploaded.as_ref().and_then(|uploaded| uploaded.outcomes.first()).cloned().unwrap();
    assert_eq!(said.result, NoteResult::Done, "her own write is no conflict: {said:?}");
    assert_eq!(server.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk\nEggs".into()));
    // Something else there (another device) still is one.
    server.with(|d| d.notes.change("VN-2", |note| note.text = "Phone".into()));
    let other = NoteOp {
        earlier: vec![NoteFields { text: Some("Milk".into()), ..Default::default() }],
        ..update("VN-2", NoteFields { text: Some("Milk\nEggs\nBread".into()), ..Default::default() }, &read)
    };
    let refused = run(&server, &request(only(other)));
    assert_eq!(outcomes(&refused)[0].0, NoteResult::Stale);
}

/// A server that wrote "Milk" into Groceries and lost its answer; the version Groceries was read at before.
fn milk_written_answer_lost() -> (StandIn, String) {
    let server = server();
    let read = version_of(&server, "VN-2");
    server.with(|d| d.lost_answers = 1);
    run(&server, &request(only(update("VN-2", NoteFields { text: Some("Milk".into()), ..Default::default() }, &read))));
    (server, read)
}

fn only(op: NoteOp) -> NotesAsk {
    NotesAsk { changes: Some(NotesUpload { account: 7, email: ME.into(), ops: vec![op] }), ..Default::default() }
}

fn text(words: &str) -> NoteFields {
    NoteFields { text: Some(words.into()), ..Default::default() }
}

#[test]
fn a_lost_answer_to_the_resend_over_her_own_write_is_tried_again_never_a_conflict() {
    let (server, read) = milk_written_answer_lost();
    // The resend over her own write loses its answer too (the server wrote it).
    server.with(|d| d.faults.push_back(Fault::later("Note/set", 1, Effect::LoseAnswer)));
    let more = NoteOp { earlier: vec![text("Milk")], ..update("VN-2", text("Milk\nEggs"), &read) };
    let again = run(&server, &request(only(more.clone())));
    assert_eq!(outcomes(&again)[0].0, NoteResult::Failed, "tried again later, her words never a copy of their own: {again:?}");
    // Nor when the note can't be read to see whose words are there.
    server.with(|d| d.faults.push_back(Fault::on("Note/get", Effect::LoseAnswer)));
    let unread = run(&server, &request(only(NoteOp { earlier: vec![text("Milk"), text("Milk\nEggs")], ..more })));
    assert_eq!(outcomes(&unread)[0].0, NoteResult::Failed, "{unread:?}");
}

#[test]
fn any_sending_unanswered_is_known_as_her_own_write() {
    let (server, read) = milk_written_answer_lost();
    // A second sending never got there (it failed before it went); a third carries both.
    let third = NoteOp { earlier: vec![text("Milk"), text("Milk\nEggs")], ..update("VN-2", text("Milk\nEggs\nBread"), &read) };
    let again = run(&server, &request(only(third)));
    assert_eq!(outcomes(&again)[0].0, NoteResult::Done, "{again:?}");
    assert_eq!(server.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk\nEggs\nBread".into()));
}

#[test]
fn a_resend_over_her_own_write_never_goes_without_the_version_the_server_says() {
    let (server, read) = milk_written_answer_lost();
    // The server's read of the note says no version of it.
    server.with(|d| d.notes.unversioned.insert("VN-2".into()));
    let more = NoteOp { earlier: vec![text("Milk")], ..update("VN-2", text("Milk\nEggs"), &read) };
    let again = run(&server, &request(only(more)));
    assert_eq!(outcomes(&again)[0].0, NoteResult::Failed, "tried again later: {again:?}");
    assert_eq!(server.with(|d| d.notes.note("VN-2").map(|note| note.text.clone())), Some("Milk".into()), "never written unchecked");
}

#[test]
fn a_resend_refused_for_now_is_tried_again_and_only_a_newer_version_is_a_conflict() {
    assert_eq!(after_resend("stateMismatch"), None, "stale, as before");
    assert_eq!(after_resend("serverFail"), Some(NoteResult::Failed));
    assert_eq!(after_resend("rateLimit"), Some(NoteResult::Failed));
    assert_eq!(after_resend("notFound"), Some(NoteResult::Gone));
    assert_eq!(after_resend("forbidden"), Some(NoteResult::Refused));
}

#[test]
fn a_whole_list_is_handed_over_even_when_a_note_of_it_can_not_be_read() {
    let server = server();
    server.with(|d| {
        d.notes.broken.insert("VN-3".into());
    });
    let result = run(&server, &request(read()));
    let got = download(&result);
    assert!(!got.complete);
    let mut listed = got.listed.clone().expect("the list was read whole");
    listed.sort();
    assert_eq!(listed, ["VN-1", "VN-2", "VN-3", "VN-4", "VN-5", "VN-6"]);
}
