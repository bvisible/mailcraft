// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Folder changes told to a server (`Mailbox/set`), over the in-process server with its answer to that call written by
//! the test: what is sent in the one call, what each change came to, the new id of a folder renamed on a server that
//! names a mailbox by where it lies, and what a server that refuses, fails or says nothing makes of them.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use mailcraft_model::sync::{ChangeOutcome, FolderChange, GraphFolder, JmapMailRequest, JmapMailResult, JmapScope};
use mailcraft_model::{FolderKind, Id};
use serde_json::{Value, json};

use crate::standin::{Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, Client, Http, Request, Response, block_on, sync_mail};

const ME: &str = "ada@example.org";

/// A server whose answer to `Mailbox/set` is the test's (`None`: the in-process server's own), and which keeps the
/// arguments of each such call.
#[derive(Clone)]
struct Script {
    inner: StandIn,
    answer: Rc<dyn Fn(&Value) -> Option<Value>>,
    asked: Rc<RefCell<Vec<Value>>>,
}

impl Http for Script {
    async fn send(&self, req: Request) -> crate::Result<Response> {
        let request: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let calls = request.get("methodCalls").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut resp = self.inner.send(req).await?;
        let Ok(mut body) = serde_json::from_slice::<Value>(&resp.body) else { return Ok(resp) };
        if let Some(responses) = body.get_mut("methodResponses").and_then(Value::as_array_mut) {
            for (call, response) in calls.iter().zip(responses.iter_mut()) {
                if call.get(0).and_then(Value::as_str) != Some("Mailbox/set") {
                    continue;
                }
                let args = call.get(1).cloned().unwrap_or(Value::Null);
                self.asked.borrow_mut().push(args.clone());
                if let Some(answer) = (self.answer)(&args) {
                    let id = response.get(2).cloned().unwrap_or(Value::Null);
                    *response = match answer.get("error").and_then(Value::as_str) {
                        Some(kind) => json!(["error", {"type": kind}, id]),
                        None => json!(["Mailbox/set", answer, id]),
                    };
                }
            }
        }
        resp.body = serde_json::to_vec(&body).unwrap();
        Ok(resp)
    }
}

/// A server of `profile` with an Inbox, an Archive and a Trash: the server, and their ids.
fn server(profile: Profile) -> (StandIn, String, String) {
    let stand_in = StandIn::new();
    let (inbox, archive) = stand_in.with(|d| {
        d.profile = profile;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let archive = d.mailbox("Archive", None, Some("archive"));
        d.mailbox("Trash", None, Some("trash"));
        (inbox, archive)
    });
    (stand_in, inbox, archive)
}

fn request(folders: Vec<FolderChange>) -> JmapMailRequest {
    JmapMailRequest {
        account: 3,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 500,
        scope: JmapScope::Light { also: Vec::new() },
        folders,
        ..Default::default()
    }
}

fn run<H: Http>(http: H, request: &JmapMailRequest) -> Vec<JmapMailResult> {
    let client = block_on(Client::connect(http, SESSION, &Auth::bearer(TOKEN))).unwrap();
    let mut pieces = Vec::new();
    block_on(sync_mail(&client, request, &AtomicBool::new(false), &mut |piece| pieces.push(piece)));
    // The answers to the folder changes come first, in one piece that has nothing else.
    for (n, piece) in pieces.iter().enumerate() {
        assert!(n == 0 || piece.folders.is_empty(), "the answers to the folder changes come first: {piece:?}");
    }
    if let Some(first) = pieces.first().filter(|piece| !piece.folders.is_empty()) {
        assert!(first.partial && first.changes.is_empty() && first.listing.is_none() && first.pages.is_empty(), "{first:?}");
    }
    pieces
}

fn scripted(stand_in: &StandIn, answer: impl Fn(&Value) -> Option<Value> + 'static) -> Script {
    Script { inner: stand_in.clone(), answer: Rc::new(answer), asked: Rc::default() }
}

fn outcomes(pieces: &[JmapMailResult]) -> Vec<(FolderChange, ChangeOutcome)> {
    pieces.iter().flat_map(|piece| piece.folders.iter()).map(|done| (done.change.clone(), done.outcome.clone())).collect()
}

fn create(folder: Id, name: &str, parent: &str, parent_folder: Option<Id>) -> FolderChange {
    FolderChange::Create { folder, name: name.into(), parent: parent.into(), parent_folder }
}

fn done(id: &str, parent: &str, name: &str) -> ChangeOutcome {
    ChangeOutcome::Done {
        folder: GraphFolder { id: id.into(), parent: parent.into(), name: name.into(), kind: FolderKind::Custom, ..Default::default() },
    }
}

fn refused(why: &str) -> ChangeOutcome {
    ChangeOutcome::Refused { why: why.into() }
}

#[test]
fn folders_made_renamed_moved_and_deleted_go_in_one_call_and_each_is_answered() {
    let (stand_in, inbox, _) = server(Profile::by_place());
    let changes = vec![
        create(10, "Plans", &inbox, None),
        create(11, "Q1", "", Some(10)),
        FolderChange::Rename { folder: 12, id: "Mold".into(), name: "New".into() },
        FolderChange::Move { folder: 12, id: "Mold".into(), parent: String::new() },
        FolderChange::Delete { id: "Mgone".into() },
    ];
    let http = scripted(&stand_in, |_| {
        Some(json!({
            "accountId": "a", "oldState": "0", "newState": "1",
            "created": {"f10": {"id": "Mp"}, "f11": {"id": "Mq"}},
            "updated": {"Mold": {"id": "Mnew"}},
            "destroyed": ["Mgone"],
        }))
    });
    let pieces = run(http.clone(), &request(changes.clone()));
    let asked = http.asked.borrow().clone();
    assert_eq!(asked.len(), 1, "one call: {asked:?}");
    let args = &asked[0];
    assert_eq!(args["create"]["f10"], json!({"name": "Plans", "parentId": inbox}));
    assert_eq!(args["create"]["f11"], json!({"name": "Q1", "parentId": "#f10"}), "under the folder the call makes, by reference");
    assert_eq!(args["update"], json!({"Mold": {"name": "New", "parentId": null}}), "one update for the folder renamed and moved");
    assert_eq!((args["destroy"].clone(), args["onDestroyRemoveEmails"].clone()), (json!(["Mgone"]), json!(true)));
    assert_eq!(
        outcomes(&pieces),
        [
            (changes[0].clone(), done("Mp", &inbox, "Plans")),
            (changes[1].clone(), done("Mq", "Mp", "Q1")),
            (changes[2].clone(), done("Mnew", "", "New")),
            (changes[3].clone(), done("Mnew", "", "")),
            (changes[4].clone(), ChangeOutcome::Done { folder: GraphFolder::default() }),
        ]
    );
    let last = pieces.last().unwrap();
    assert!(last.errors.is_empty() && last.whole, "{last:?}");
}

#[test]
fn a_folder_that_takes_another_id_is_read_in_a_light_run() {
    let (stand_in, inbox, archive) = server(Profile::by_place());
    // The server says the folder renamed is the Archive now (as a server that names a mailbox by its path would).
    let to = archive.clone();
    let http = scripted(&stand_in, move |_| Some(json!({"accountId": "a", "newState": "1", "updated": {"Mold": {"id": to}}})));
    let pieces = run(http, &request(vec![FolderChange::Rename { folder: 12, id: "Mold".into(), name: "Archive".into() }]));
    assert_eq!(outcomes(&pieces).len(), 1);
    let read: Vec<String> = stand_in.asked().iter().filter(|a| a.method == "Email/query").filter_map(|a| a.mailbox.clone()).collect();
    assert_eq!(read, [inbox, archive], "the Inbox, then the folder that took another id");
}

#[test]
fn what_the_server_refuses_fails_or_says_nothing_of_is_told_apart() {
    let (stand_in, inbox, _) = server(Profile::by_place());
    let changes = vec![
        create(10, "Plans", &inbox, None),
        // Under a folder this call doesn't make: it waits, and isn't sent.
        create(20, "Later", "", Some(99)),
        FolderChange::Rename { folder: 12, id: "Mold".into(), name: "New".into() },
        FolderChange::Delete { id: "Mgone".into() },
        FolderChange::Delete { id: "Mquiet".into() },
        // Ids that can't be a server's are refused without a word to it.
        FolderChange::Rename { folder: 13, id: "M x".into(), name: "Bad".into() },
    ];
    let http = scripted(&stand_in, |_| {
        Some(json!({
            "accountId": "a", "newState": "1",
            "notCreated": {"f10": {"type": "forbidden", "description": "<script>no</script>"}},
            "notUpdated": {"Mold": {"type": "notFound"}},
            "notDestroyed": {"Mgone": {"type": "serverFail"}},
            // A key that was not sent is not looked at.
            "created": {"f20": {"id": "Mx"}},
        }))
    });
    let pieces = run(http.clone(), &request(changes.clone()));
    let args = http.asked.borrow()[0].clone();
    assert!(args["create"].get("f20").is_none(), "{args}");
    assert_eq!(args["destroy"], json!(["Mgone", "Mquiet"]));
    assert_eq!(
        outcomes(&pieces),
        [(changes[0].clone(), refused("forbidden")), (changes[2].clone(), ChangeOutcome::Gone), (changes[5].clone(), refused("invalidId"))]
    );
    let last = pieces.last().unwrap();
    assert!(!last.errors.iter().any(|e| e.contains("script")), "{:?}", last.errors);
    assert!(last.errors.iter().any(|e| e.contains("Couldn't send 3 changes")), "{:?}", last.errors);
}

#[test]
fn a_server_that_has_no_mailbox_set_refuses_every_change() {
    let (stand_in, inbox, _) = server(Profile::standard());
    let changes = vec![create(10, "Plans", &inbox, None), FolderChange::Delete { id: "Mgone".into() }];
    let pieces = run(stand_in.clone(), &request(changes.clone()));
    assert_eq!(outcomes(&pieces), [(changes[0].clone(), refused("unknownMethod")), (changes[1].clone(), refused("unknownMethod"))]);
    // The run goes on: the Inbox is read.
    assert!(pieces.iter().any(|piece| !piece.pages.is_empty()));
}

#[test]
fn a_call_that_fails_leaves_every_change_for_the_next_run_and_the_run_goes_on() {
    let (stand_in, inbox, _) = server(Profile::by_place());
    let http = scripted(&stand_in, |_| Some(json!({"error": "serverFail"})));
    let pieces = run(http, &request(vec![create(10, "Plans", &inbox, None)]));
    assert!(outcomes(&pieces).is_empty());
    let last = pieces.last().unwrap();
    assert!(last.errors.iter().any(|e| e.starts_with("Couldn't send 1 change")), "{:?}", last.errors);
    assert!(pieces.iter().any(|piece| !piece.pages.is_empty()));
}

#[test]
fn no_folder_change_asks_nothing() {
    let (stand_in, ..) = server(Profile::by_place());
    let http = scripted(&stand_in, |_| None);
    let pieces = run(http.clone(), &request(Vec::new()));
    assert!(http.asked.borrow().is_empty() && outcomes(&pieces).is_empty());
}
