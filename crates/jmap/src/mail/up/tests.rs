// //// Neoffice — added file (no upstream equivalent)
//! The changes of a request told to a server, over the in-process server of both kinds and over servers
//! that say what a test has them say: what is sent and in how many calls, what each change came to, how
//! a move's new id is learnt, and what a failing or a hostile server makes of the run.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::sync::{JmapChange, JmapMailRequest, JmapMailResult, JmapOutcome, JmapScope};
use proptest::prelude::*;
use serde_json::{Value, json};

use crate::standin::{Profile, SESSION, StandIn, TOKEN};
use crate::{Auth, Client, Http, Request, Response, block_on, sync_mail};

const ME: &str = "ada@example.org";
const ID: u64 = 3;

/// A server with an Inbox of emails `m1`… (the first the oldest), an Archive and a Trash.
struct World {
    stand_in: StandIn,
    inbox: String,
    archive: String,
    trash: String,
    /// The emails' ids, `m1` first.
    mails: Vec<String>,
}

fn mail(subject: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// When the email numbered `n` arrives: later for a bigger `n`.
fn at(n: usize) -> String {
    format!("2026-01-01T{:02}:{:02}:{:02}Z", n / 3600, (n % 3600) / 60, n % 60)
}

/// A server of `profile` with `n` emails in its Inbox, named `m1` to `m{n}`; the keywords of the email `k` (from 1)
/// are `keywords(k)`.
fn world_with(profile: Profile, n: usize, keywords: impl Fn(usize) -> Vec<&'static str>) -> World {
    let stand_in = StandIn::new();
    let (inbox, archive, trash, mails) = stand_in.with(|d| {
        d.profile = profile;
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        let archive = d.mailbox("Archive", None, Some("archive"));
        let trash = d.mailbox("Trash", None, Some("trash"));
        let mails = (1..=n).map(|k| d.deliver(&inbox, &mail(&format!("m{k}")), &keywords(k), &at(k))).collect();
        (inbox, archive, trash, mails)
    });
    World { stand_in, inbox, archive, trash, mails }
}

fn world(profile: Profile, n: usize) -> World {
    world_with(profile, n, |_| Vec::new())
}

impl World {
    /// The email `k` (from 1), by its subject, wherever it lies and whatever its id is now: its keywords and its
    /// mailboxes' names.
    fn state_of(&self, k: usize) -> Option<(Vec<String>, Vec<String>)> {
        let raw = mail(&format!("m{k}"));
        self.stand_in.with(|d| {
            let email = d.emails.iter().find(|e| e.raw == raw)?;
            let names = email.mailboxes.iter().filter_map(|id| d.mailboxes.iter().find(|m| m.id == *id)).map(|m| m.name.clone()).collect();
            Some((email.keywords.iter().cloned().collect(), names))
        })
    }

    /// The id the server has for the email `k` now.
    fn id_now(&self, k: usize) -> Option<String> {
        let raw = mail(&format!("m{k}"));
        self.stand_in.with(|d| d.emails.iter().find(|e| e.raw == raw).map(|e| e.id.clone()))
    }
}

fn request(changes: Vec<JmapChange>, also: &[&str]) -> JmapMailRequest {
    JmapMailRequest {
        account: ID,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 500,
        scope: JmapScope::Light { also: also.iter().map(|id| id.to_string()).collect() },
        changes,
        ..Default::default()
    }
}

fn marks(id: &str, read: Option<bool>, flagged: Option<bool>, answered: Option<bool>, forwarded: Option<bool>) -> JmapChange {
    JmapChange::Keywords { message: 1, id: id.into(), read, flagged, answered, forwarded }
}

fn read(id: &str, on: bool) -> JmapChange {
    marks(id, Some(on), None, None, None)
}

fn moved(id: &str, from: &str, to: &str) -> JmapChange {
    JmapChange::Move { message: 1, id: id.into(), from: from.into(), to: to.into() }
}

fn destroy(id: &str) -> JmapChange {
    JmapChange::Destroy { id: id.into() }
}

fn done(id: &str) -> JmapOutcome {
    JmapOutcome::Done { id: id.into() }
}

fn refused(why: &str) -> JmapOutcome {
    JmapOutcome::Refused { why: why.into() }
}

fn run_over<H: Http>(http: H, request: &JmapMailRequest, cancel: &AtomicBool) -> Vec<JmapMailResult> {
    let client = block_on(Client::connect(http, SESSION, &Auth::bearer(TOKEN))).unwrap();
    let mut pieces = Vec::new();
    block_on(sync_mail(&client, request, cancel, &mut |piece| pieces.push(piece)));
    shaped(request, &pieces);
    pieces
}

fn run(stand_in: &StandIn, request: &JmapMailRequest) -> Vec<JmapMailResult> {
    run_over(stand_in.clone(), request, &AtomicBool::new(false))
}

/// What every run is like with changes in it: the pieces are for the account, all `partial` but the last; the
/// answers to the changes come in one piece, the first, which has nothing else; each answered change is one that
/// was asked, once, in the order it was asked.
fn shaped(request: &JmapMailRequest, pieces: &[JmapMailResult]) {
    let (last, rest) = pieces.split_last().expect("a run ends with a piece");
    assert!(!last.partial && last.changes.is_empty(), "{last:?}");
    assert!(rest.iter().all(|piece| piece.partial && (piece.account, piece.email.as_str()) == (ID, ME)), "{rest:?}");
    assert!(pieces.iter().skip(1).all(|piece| piece.changes.is_empty()), "the answers to the changes come first, in one piece");
    if let Some(first) = pieces.first().filter(|piece| !piece.changes.is_empty()) {
        assert!(first.partial && first.listing.is_none() && first.pages.is_empty() && first.progress.is_none(), "{first:?}");
        let mut asked = request.changes.iter();
        for done in &first.changes {
            assert!(asked.any(|change| *change == done.change), "an answer to a change nobody asked, or out of order: {done:?}");
        }
    }
}

/// What the changes came to, in the order they were answered.
fn outcomes(pieces: &[JmapMailResult]) -> Vec<(JmapChange, JmapOutcome)> {
    pieces.iter().flat_map(|piece| piece.changes.iter()).map(|done| (done.change.clone(), done.outcome.clone())).collect()
}

fn only_outcomes(pieces: &[JmapMailResult]) -> Vec<JmapOutcome> {
    outcomes(pieces).into_iter().map(|(_, outcome)| outcome).collect()
}

fn last(pieces: &[JmapMailResult]) -> &JmapMailResult {
    pieces.last().expect("a run ends with a piece")
}

/// What `Email/set` calls it was asked to make, in order: how many it was to update and to destroy.
fn sets(stand_in: &StandIn) -> Vec<(usize, usize)> {
    stand_in.asked().iter().filter(|a| a.method == "Email/set").map(|a| (a.updates.unwrap_or(0), a.destroys.unwrap_or(0))).collect()
}

/// A server that runs a request and lets a test change the answer: it is given what the request asked and what
/// the answer was, and says what the answer is to be.
#[derive(Clone)]
struct Rewrite {
    inner: StandIn,
    with: Rc<dyn Fn(&Value, Value) -> Value>,
}

impl Http for Rewrite {
    async fn send(&self, req: Request) -> crate::Result<Response> {
        let asked: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let mut resp = self.inner.send(req).await?;
        if resp.status == 200
            && let Ok(body) = serde_json::from_slice::<Value>(&resp.body)
        {
            resp.body = serde_json::to_vec(&(self.with)(&asked, body)).unwrap();
        }
        Ok(resp)
    }
}

/// What a test makes of the answer to an `Email/set`.
enum Say {
    /// This is its result.
    Result(Value),
    /// The call failed with this method error.
    Error(&'static str),
    /// There is no answer to it.
    Missing,
}

/// A server that answers every `Email/set` as `f` says (given its arguments and its result as they were).
fn saying(stand_in: &StandIn, f: impl Fn(&Value, Value) -> Say + 'static) -> Rewrite {
    let with = move |asked: &Value, mut body: Value| {
        let calls = asked.get("methodCalls").and_then(Value::as_array).cloned().unwrap_or_default();
        let Some(responses) = body.get_mut("methodResponses").and_then(Value::as_array_mut) else { return body };
        let mut kept = Vec::new();
        for (n, response) in responses.iter().enumerate() {
            if response[0] != "Email/set" {
                kept.push(response.clone());
                continue;
            }
            let args = calls.get(n).map(|call| call[1].clone()).unwrap_or(Value::Null);
            match f(&args, response[1].clone()) {
                Say::Result(result) => kept.push(json!(["Email/set", result, response[2]])),
                Say::Error(kind) => kept.push(json!(["error", {"type": kind, "description": "a test says so"}, response[2]])),
                Say::Missing => {}
            }
        }
        *responses = kept;
        body
    };
    Rewrite { inner: stand_in.clone(), with: Rc::new(with) }
}

#[test]
fn the_marks_that_changed_go_up_as_patches_of_those_marks_alone_and_the_rest_of_the_keywords_stay() {
    for profile in [Profile::standard(), Profile::by_place()] {
        let w = world_with(profile, 4, |k| match k {
            1 => vec!["$seen", "$custom"],
            3 => vec!["$flagged", "$draft"],
            _ => Vec::new(),
        });
        let ids = &w.mails;
        let changes = vec![
            marks(&ids[0], Some(false), Some(true), None, None),
            marks(&ids[1], Some(true), None, Some(true), None),
            marks(&ids[2], None, Some(false), None, Some(true)),
        ];
        let pieces = run(&w.stand_in, &request(changes.clone(), &[]));

        assert_eq!(only_outcomes(&pieces), [done(""), done(""), done("")]);
        assert_eq!(outcomes(&pieces).into_iter().map(|(change, _)| change).collect::<Vec<_>>(), changes);
        assert_eq!(w.state_of(1).unwrap().0, ["$custom", "$flagged"], "unread, flagged; the keyword it was not told of stays");
        assert_eq!(w.state_of(2).unwrap().0, ["$answered", "$seen"]);
        assert_eq!(w.state_of(3).unwrap().0, ["$draft", "$forwarded"], "$draft is not a mark the program changes");
        assert!(w.state_of(4).unwrap().0.is_empty(), "an email not named is not touched");
        // One call for the three, made before the mailboxes are read, and nothing asked about ids: nothing moved.
        assert_eq!(sets(&w.stand_in), [(3, 0)]);
        assert_eq!(w.stand_in.calls()[0], "Email/set");
        let first = w.stand_in.seen().into_iter().find(|s| s.path.ends_with("/api")).unwrap();
        assert_eq!(first.calls, ["Email/set"], "{profile:?}");
        // What is read afterwards is the server after the changes.
        let end = last(&pieces);
        assert!(end.whole && end.errors.is_empty() && !end.stopped, "{end:?}");
        let page = pieces.iter().flat_map(|piece| piece.pages.iter()).find(|page| page.folder == w.inbox).unwrap();
        let read_back: Vec<(bool, bool)> =
            page.messages.iter().rev().map(|m| (m.server.read, m.server.flag == mailcraft_model::FlagStatus::Flagged)).collect();
        assert_eq!(read_back, [(false, true), (true, false), (false, false), (false, false)]);
    }
}

#[test]
fn the_marks_of_a_message_are_made_before_its_move_and_a_server_that_keeps_its_ids_keeps_the_id() {
    let w = world(Profile::standard(), 3);
    let ids = &w.mails;
    let changes = vec![read(&ids[0], true), moved(&ids[0], &w.inbox, &w.archive), moved(&ids[1], &w.inbox, &w.trash)];
    let pieces = run(&w.stand_in, &request(changes, &[&w.archive]));

    // The id stayed, which the server showed when it was asked for the moved ids by the ones they had.
    assert_eq!(only_outcomes(&pieces), [done(""), done(&ids[0]), done(&ids[1])]);
    assert_eq!(w.state_of(1).unwrap(), (vec!["$seen".to_string()], vec!["Archive".to_string()]));
    assert_eq!(w.state_of(2).unwrap().1, ["Trash"]);
    assert_eq!(w.state_of(3).unwrap().1, ["Inbox"]);
    // Two calls: the mark of the first, under the id it has before it moves; then the two moves.
    assert_eq!(sets(&w.stand_in), [(1, 0), (2, 0)], "a mark and a move are two updates");
    let api: Vec<_> = w.stand_in.seen().into_iter().filter(|s| s.path.ends_with("/api")).collect();
    assert_eq!(api[0].calls, ["Email/set"], "the marks are made alone");
    assert_eq!(api[1].calls, ["Email/set", "Email/get"], "the ids of the moved are asked for in the same request");
    assert_eq!((api[1].asked[1].ids, api[1].asked[1].properties.clone()), (Some(2), vec!["id".to_string()]));
    // What is read afterwards: the Inbox without the two, and the Archive with the one, under its id.
    let page = |mailbox: &str| pieces.iter().flat_map(|piece| piece.pages.iter()).find(|page| page.folder == mailbox).unwrap();
    assert!(page(&w.inbox).afresh && page(&w.inbox).messages.len() == 1);
    assert_eq!(page(&w.archive).messages.iter().map(|m| m.server.id.clone()).collect::<Vec<_>>(), [ids[0].clone()]);
    assert!(last(&pieces).whole);
}

#[test]
fn a_move_the_server_refuses_leaves_the_marks_of_the_same_message_made() {
    // The Archive is full: the move is refused, and the read mark, which is its own update, is not taken with it.
    let w = world(Profile::standard(), 2);
    let ids = w.mails.clone();
    w.stand_in.with(|d| d.refused_moves.insert(ids[0].clone(), "overQuota"));
    let changes = vec![read(&ids[1], true), moved(&ids[0], &w.inbox, &w.archive), read(&ids[0], true)];
    let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
    let got = outcomes(&pieces);
    assert_eq!(got.len(), 3);
    // The marks of both messages are made; only the move is refused, and it is the one that is said so (the outcomes come
    // in the order the changes were asked).
    assert_eq!(got, vec![(changes[0].clone(), done("")), (changes[1].clone(), refused("overQuota")), (changes[2].clone(), done(""))]);
    assert_eq!(
        (w.state_of(1).unwrap(), w.state_of(2).unwrap().0),
        ((vec!["$seen".to_string()], vec!["Inbox".to_string()]), vec!["$seen".to_string()])
    );
    assert!(last(&pieces).whole);
}

#[test]
fn a_server_that_names_mail_by_place_gives_a_moved_message_an_id_it_does_not_say_so_the_outcome_has_none() {
    let w = world(Profile::by_place(), 3);
    let ids = w.mails.clone();
    let changes = vec![read(&ids[0], true), moved(&ids[0], &w.inbox, &w.archive), moved(&ids[1], &w.inbox, &w.trash)];
    let pieces = run(&w.stand_in, &request(changes, &[&w.archive]));

    // Neither old id is there any more, and nobody said the new ones.
    assert_eq!(only_outcomes(&pieces), [done(""), done(""), done("")]);
    assert_eq!(w.state_of(1).unwrap(), (vec!["$seen".to_string()], vec!["Archive".to_string()]));
    assert_ne!(w.id_now(1).unwrap(), ids[0]);
    // The window of the Archive, read in the same run, has it under its new id, for the engine to find again.
    let archive = pieces.iter().flat_map(|piece| piece.pages.iter()).find(|page| page.folder == w.archive).unwrap();
    assert_eq!(
        archive.messages.iter().map(|m| (m.server.id.clone(), m.message_id.clone())).collect::<Vec<_>>(),
        [(w.id_now(1).unwrap(), "m1@example.org".to_string())]
    );
    // A server that does say which id it gave: that is the outcome's.
    let w = world(Profile { says_moved_id: true, ..Profile::by_place() }, 2);
    let pieces = run(&w.stand_in, &request(vec![moved(&w.mails[0], &w.inbox, &w.archive), read(&w.mails[1], true)], &[]));
    assert_eq!(only_outcomes(&pieces), [done(&w.id_now(1).unwrap()), done("")]);
    assert_ne!(w.id_now(1).unwrap(), w.mails[0]);
    assert!(last(&pieces).whole);
}

#[test]
fn a_server_that_takes_one_call_to_a_request_is_not_asked_for_the_ids_and_the_moved_message_has_none() {
    let w = world(Profile::standard(), 2);
    w.stand_in.with(|d| d.max_calls = 1);
    let pieces = run(&w.stand_in, &request(vec![moved(&w.mails[0], &w.inbox, &w.archive)], &[]));
    assert_eq!(only_outcomes(&pieces), [done("")], "its id is not known to be what it was");
    assert!(w.stand_in.seen().iter().all(|s| s.calls.len() <= 1));
    assert_eq!(w.state_of(1).unwrap().1, ["Archive"]);
    assert!(last(&pieces).whole);
}

#[test]
fn a_call_holds_no_more_objects_than_the_session_allows_and_never_more_than_500() {
    // The marks, then the destroys, in calls of the size the session names.
    let w = world(Profile::standard(), 14);
    w.stand_in.with(|d| d.max_set = 4);
    let ids = &w.mails;
    let mut changes: Vec<JmapChange> = ids[..7].iter().map(|id| read(id, true)).collect();
    changes.extend(ids[7..].iter().map(|id| destroy(id)));
    let pieces = run(&w.stand_in, &request(changes, &[]));
    assert_eq!(sets(&w.stand_in), [(4, 0), (3, 0), (0, 4), (0, 3)]);
    assert!(only_outcomes(&pieces).iter().all(|outcome| *outcome == done("")) && only_outcomes(&pieces).len() == 14);
    assert!((1..=7).all(|k| w.state_of(k).unwrap().0 == ["$seen"]) && (8..=14).all(|k| w.state_of(k).is_none()));
    assert!(last(&pieces).whole);

    // A session that says it takes 1000 is believed up to 500.
    let w = world(Profile::standard(), 1_200);
    w.stand_in.with(|d| d.max_set = 1_000);
    let client = block_on(Client::connect(w.stand_in.clone(), SESSION, &Auth::bearer(TOKEN))).unwrap();
    assert_eq!(client.max_objects_in_set(), 1_000);
    let changes: Vec<JmapChange> = w.mails.iter().map(|id| destroy(id)).collect();
    let pieces = run(&w.stand_in, &request(changes, &[]));
    assert_eq!(sets(&w.stand_in), [(0, 500), (0, 500), (0, 200)]);
    assert_eq!(only_outcomes(&pieces).len(), 1_200);
    assert!(w.stand_in.with(|d| d.emails.is_empty()));
    // The answers came in one piece.
    assert_eq!(pieces.iter().filter(|piece| !piece.changes.is_empty()).count(), 1);
}

#[test]
fn each_change_has_its_own_outcome_and_what_the_server_could_not_do_just_now_is_none() {
    let w = world(Profile::standard(), 8);
    let ids = w.mails.clone();
    w.stand_in.with(|d| {
        d.refused_sets.insert(ids[0].clone(), "forbidden");
        d.refused_sets.insert(ids[2].clone(), "serverFail");
        d.refused_sets.insert(ids[3].clone(), "rateLimit");
        d.refused_sets.insert(ids[4].clone(), "overQuota");
        d.refused_sets.insert(ids[7].clone(), "serverUnavailable");
        d.remove(&ids[1]);
        d.remove(&ids[6]);
    });
    let changes = vec![
        read(&ids[0], true),
        read(&ids[1], true),
        read(&ids[2], true),
        read(&ids[3], true),
        destroy(&ids[4]),
        moved(&ids[5], &w.inbox, &w.archive),
        destroy(&ids[6]),
        read(&ids[7], true),
    ];
    let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
    let got = outcomes(&pieces);
    let expected = vec![
        (changes[0].clone(), refused("forbidden")),
        (changes[1].clone(), JmapOutcome::Gone),
        (changes[4].clone(), refused("overQuota")),
        (changes[5].clone(), done(&ids[5])),
        (changes[6].clone(), JmapOutcome::Gone),
    ];
    assert_eq!(got, expected, "serverFail, rateLimit and serverUnavailable are no answer: the change waits");
    let end = last(&pieces);
    // Marks that wait change nothing a window says: the run is whole all the same, and says what waits.
    assert!(end.whole && !end.stopped && !end.auth_refused && end.busy_for.is_none(), "{end:?}");
    assert_eq!(end.errors, ["Couldn't send 3 changes to the server: the server could not make them just now (serverFail)"]);
    // The run went on to read the mailboxes all the same.
    assert!(pieces.iter().any(|piece| piece.pages.iter().any(|page| page.folder == w.inbox)));
    // A refusal changed nothing on the server; the move went through.
    assert!(w.state_of(1).unwrap().0.is_empty());
    assert_eq!(w.state_of(6).unwrap().1, ["Archive"]);
}

#[test]
fn a_set_error_is_kept_as_its_type_alone_in_letters() {
    let w = world(Profile::standard(), 3);
    let ids = w.mails.clone();
    let server = saying(&w.stand_in, {
        let ids = ids.clone();
        move |_, _| {
            Say::Result(json!({
                "updated": {},
                "notUpdated": {
                    &ids[0]: {"type": "bad\u{0}type\r\n!! <b>x</b>", "description": "ada@example.org may not do this"},
                    &ids[1]: {"description": "no type"},
                    &ids[2]: "not even an object",
                },
            }))
        }
    });
    let pieces = run_over(server, &request(vec![read(&ids[0], true), read(&ids[1], true), read(&ids[2], true)], &[]), &AtomicBool::new(false));
    assert_eq!(only_outcomes(&pieces), [refused("badtypebxb"), refused("unknown"), refused("unknown")]);
    let shown = format!("{pieces:?}");
    assert!(!shown.contains("may not do this") && !shown.contains('@'), "{shown}");
    let long = saying(&w.stand_in, {
        let id = ids[0].clone();
        move |_, _| Say::Result(json!({"notUpdated": {&id: {"type": "t".repeat(5_000)}}}))
    });
    let pieces = run_over(long, &request(vec![read(&ids[0], true)], &[]), &AtomicBool::new(false));
    assert_eq!(only_outcomes(&pieces), [refused(&"t".repeat(40))]);
}

#[test]
fn a_call_refused_as_a_whole_because_the_account_cannot_be_changed_refuses_every_change_in_it() {
    for kind in ["accountReadOnly", "forbidden", "accountNotSupportedByMethod", "unknownMethod"] {
        let w = world(Profile::standard(), 3);
        let server = saying(&w.stand_in, move |_, _| Say::Error(kind));
        let changes = vec![read(&w.mails[0], true), moved(&w.mails[1], &w.inbox, &w.archive), destroy(&w.mails[2])];
        let pieces = run_over(server, &request(changes, &[]), &AtomicBool::new(false));
        assert_eq!(only_outcomes(&pieces), [refused(kind), refused(kind), refused(kind)], "{kind}");
        assert!(last(&pieces).whole && last(&pieces).errors.is_empty(), "{kind}: a refusal is an answer");
    }
}

#[test]
fn a_call_that_fails_in_another_way_waits_and_the_run_goes_on_to_read() {
    for kind in ["serverFail", "serverUnavailable", "invalidArguments", "stateMismatch", "serverPartialFail"] {
        let w = world(Profile::standard(), 3);
        let server = saying(&w.stand_in, move |_, _| Say::Error(kind));
        let pieces = run_over(server, &request(vec![read(&w.mails[0], true), read(&w.mails[1], true)], &[]), &AtomicBool::new(false));
        assert!(only_outcomes(&pieces).is_empty(), "{kind}");
        let end = last(&pieces);
        // (Marks that wait change nothing a window says: the run is whole all the same.)
        assert!(end.whole && !end.stopped && end.busy_for.is_none() && !end.auth_refused, "{kind}: {end:?}");
        assert_eq!(end.errors.len(), 1, "{kind}: {:?}", end.errors);
        assert!(end.errors[0].starts_with("Couldn't send 2 changes to the server: the server refused: "), "{kind}: {}", end.errors[0]);
        assert!(!end.errors[0].contains('@'), "{}", end.errors[0]);
        assert!(pieces.iter().any(|piece| !piece.pages.is_empty()), "{kind}: the Inbox is read all the same");
    }
    // A call that has no answer at all is the same: the change waits.
    let w = world(Profile::standard(), 2);
    let server = saying(&w.stand_in, |_, _| Say::Missing);
    let pieces = run_over(server, &request(vec![read(&w.mails[0], true)], &[]), &AtomicBool::new(false));
    assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole);
    assert_eq!(last(&pieces).errors.len(), 1);
    // A move that waits is another matter: it may have been made, and no window of the mailbox it left lists the message.
    let w = world(Profile::standard(), 2);
    let server = saying(&w.stand_in, |_, _| Say::Missing);
    let pieces = run_over(server, &request(vec![moved(&w.mails[0], &w.inbox, &w.archive)], &[]), &AtomicBool::new(false));
    assert!(only_outcomes(&pieces).is_empty() && !last(&pieces).whole);
    assert_eq!(last(&pieces).errors.len(), 1);
}

#[test]
fn the_ids_of_the_moved_are_read_back_in_calls_the_server_takes_whatever_it_takes_to_set() {
    // A server that takes 500 objects to a `set` and 3 to a `get`: a call holds no more moves than it may be asked about.
    let w = world(Profile::standard(), 7);
    w.stand_in.with(|d| d.max_get = 3);
    let changes: Vec<JmapChange> = w.mails.iter().map(|id| moved(id, &w.inbox, &w.archive)).collect();
    let pieces = run(&w.stand_in, &request(changes, &[]));
    assert_eq!(sets(&w.stand_in), [(3, 0), (3, 0), (1, 0)]);
    // This server keeps the id of a message that moves, and says so: every one was found under its own.
    assert_eq!(only_outcomes(&pieces), w.mails.iter().map(|id| done(id)).collect::<Vec<_>>());
    assert!(last(&pieces).whole && last(&pieces).errors.is_empty(), "{:?}", last(&pieces));
}

#[test]
fn only_a_move_that_may_have_been_made_without_a_word_keeps_a_run_from_being_whole() {
    // The server says, of the move itself, that it was asked too often: it has not begun, it is said, and the run is whole.
    let w = world(Profile::standard(), 3);
    let id = w.mails[0].clone();
    w.stand_in.with(|d| d.refused_sets.insert(id.clone(), "rateLimit"));
    let pieces = run(&w.stand_in, &request(vec![moved(&id, &w.inbox, &w.archive)], &[&w.archive]));
    assert!(only_outcomes(&pieces).is_empty());
    assert!(last(&pieces).whole, "{:?}", last(&pieces));
    assert_eq!(last(&pieces).errors, ["Couldn't send 1 change to the server: the server could not make them just now (rateLimit)"]);
    assert_eq!(w.state_of(1).unwrap().1, ["Inbox"]);

    // A failure of the server's own, of the move: it may have failed after it began (a connection lost when the move was
    // made), so the message may be where it was moved to and no window of the mailbox it left would list it.
    for kind in ["serverFail", "serverUnavailable"] {
        let w = world(Profile::standard(), 3);
        let id = w.mails[0].clone();
        w.stand_in.with(|d| d.refused_sets.insert(id.clone(), kind));
        let pieces = run(&w.stand_in, &request(vec![moved(&id, &w.inbox, &w.archive)], &[&w.archive]));
        assert!(only_outcomes(&pieces).is_empty() && !last(&pieces).whole, "{kind}: {:?}", last(&pieces));
        assert_eq!(last(&pieces).errors, [format!("Couldn't send 1 change to the server: the server could not make them just now ({kind})")]);
        // (A mark with the same answer changes nothing a window says.)
        let w = world(Profile::standard(), 3);
        let id = w.mails[0].clone();
        w.stand_in.with(|d| d.refused_sets.insert(id.clone(), kind));
        let pieces = run(&w.stand_in, &request(vec![read(&id, true)], &[]));
        assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole, "{kind}: {:?}", last(&pieces));
    }

    // The mailbox it was to go to is one the server no longer has (another client removed it, and the run's own listing
    // of the mailboxes says so): the move can not have been made, whatever the server said, and the run is whole.
    for kind in ["serverFail", "serverUnavailable"] {
        let w = world(Profile::standard(), 3);
        let id = w.mails[0].clone();
        w.stand_in.with(|d| {
            d.refused_sets.insert(id.clone(), kind);
            d.mailboxes.retain(|m| m.id != w.archive);
        });
        let pieces = run(&w.stand_in, &request(vec![moved(&id, &w.inbox, &w.archive)], &[]));
        assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole, "{kind}: {:?}", last(&pieces));
        assert_eq!(last(&pieces).errors.len(), 1, "it is still said: {:?}", last(&pieces).errors);
    }

    // A method error is the same: the call was not made. (Except `serverPartialFail`: some of it was.)
    for (kind, whole) in [("serverFail", true), ("serverUnavailable", true), ("serverPartialFail", false)] {
        let w = world(Profile::standard(), 3);
        let server = saying(&w.stand_in, move |_, _| Say::Error(kind));
        let pieces = run_over(server, &request(vec![moved(&w.mails[0], &w.inbox, &w.archive)], &[]), &AtomicBool::new(false));
        assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole == whole, "{kind}: {:?}", last(&pieces));
    }

    // An answer that leaves the move out of both lists may be a move that was made.
    let w = world(Profile::standard(), 3);
    let server = saying(&w.stand_in, |_, _| Say::Result(json!({"updated": {}, "notUpdated": {}})));
    let pieces = run_over(server, &request(vec![moved(&w.mails[0], &w.inbox, &w.archive)], &[]), &AtomicBool::new(false));
    assert!(only_outcomes(&pieces).is_empty() && !last(&pieces).whole, "{:?}", last(&pieces));
    // (And a mark left out of it may not: it changes nothing a window says.)
    let w = world(Profile::standard(), 3);
    let server = saying(&w.stand_in, |_, _| Say::Result(json!({"updated": {}, "notUpdated": {}})));
    let pieces = run_over(server, &request(vec![read(&w.mails[0], true)], &[]), &AtomicBool::new(false));
    assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole, "{:?}", last(&pieces));

    // A mark the server cannot make just now waits, and the move of another message goes through: the run is whole.
    let w = world(Profile::standard(), 3);
    let id = w.mails[0].clone();
    w.stand_in.with(|d| d.refused_sets.insert(id.clone(), "serverFail"));
    let pieces = run(&w.stand_in, &request(vec![read(&id, true), moved(&w.mails[1], &w.inbox, &w.archive)], &[&w.archive]));
    assert_eq!(only_outcomes(&pieces), [done(&w.mails[1])]);
    assert!(last(&pieces).whole, "{:?}", last(&pieces));
    assert_eq!((w.state_of(1).unwrap().0.is_empty(), w.state_of(2).unwrap().1), (true, vec!["Archive".to_string()]));

    // A call that fails as a whole (nothing answered) does not go on to ask for the moves: they were not made.
    let w = world(Profile::standard(), 3);
    let server = saying(&w.stand_in, |_, _| Say::Error("serverFail"));
    let pieces = run_over(server, &request(vec![read(&w.mails[0], true), moved(&w.mails[1], &w.inbox, &w.archive)], &[]), &AtomicBool::new(false));
    assert!(only_outcomes(&pieces).is_empty() && last(&pieces).whole, "{:?}", last(&pieces));
    assert_eq!(sets(&w.stand_in), [(1, 0)], "the call for the marks was the only one that was made");
}

#[test]
fn a_call_the_server_finds_too_large_is_halved_until_it_takes_it() {
    let w = world(Profile::standard(), 9);
    // The session says 500; the server takes 2.
    w.stand_in.with(|d| d.max_set = 2);
    let server = Rewrite {
        inner: w.stand_in.clone(),
        with: Rc::new(|asked: &Value, mut body: Value| {
            if asked.is_null()
                && let Some(core) = body.pointer_mut("/capabilities/urn:ietf:params:jmap:core")
            {
                core["maxObjectsInSet"] = json!(500);
            }
            body
        }),
    };
    let changes: Vec<JmapChange> = w.mails.iter().map(|id| read(id, true)).collect();
    let pieces = run_over(server, &request(changes, &[]), &AtomicBool::new(false));
    assert_eq!(only_outcomes(&pieces).len(), 9);
    assert!(only_outcomes(&pieces).iter().all(|outcome| *outcome == done("")));
    // Nine are refused; so are the first four of them, and then two and two go through; the other five are refused,
    // and so are three of them, which are made as one and two.
    assert_eq!(sets(&w.stand_in), [(9, 0), (4, 0), (2, 0), (2, 0), (5, 0), (2, 0), (3, 0), (1, 0), (2, 0)]);
    assert!(last(&pieces).whole, "{:?}", last(&pieces));
    assert!((1..=9).all(|k| w.state_of(k).unwrap().0 == ["$seen"]));
}

#[test]
fn a_busy_server_a_refused_sign_in_or_a_cancel_ends_the_run_and_what_was_answered_is_handed_over() {
    // Two calls of two; the second is told to wait.
    let w = world(Profile::standard(), 4);
    w.stand_in.with(|d| {
        d.max_set = 2;
        d.refuse_next.extend([None, Some((503, Some(7)))]);
    });
    let changes: Vec<JmapChange> = w.mails.iter().map(|id| read(id, true)).collect();
    let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
    assert_eq!(outcomes(&pieces).into_iter().map(|(change, _)| change).collect::<Vec<_>>(), changes[..2]);
    let end = last(&pieces);
    assert_eq!((end.busy_for, end.whole, end.pages.len()), (Some(7), false, 0), "{end:?}");
    assert!(pieces.iter().all(|piece| piece.pages.is_empty()), "nothing is read from a server that asked to be left alone");
    assert!((1..=2).all(|k| w.state_of(k).unwrap().0 == ["$seen"]) && (3..=4).all(|k| w.state_of(k).unwrap().0.is_empty()));

    // The token goes after the first call.
    let w = world(Profile::standard(), 4);
    w.stand_in.with(|d| {
        d.max_set = 2;
        d.drop_token_after = Some(1);
    });
    let changes: Vec<JmapChange> = w.mails.iter().map(|id| read(id, true)).collect();
    let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
    assert_eq!(outcomes(&pieces).len(), 2);
    assert!(last(&pieces).auth_refused && !last(&pieces).whole);

    // A cancel is read before every call.
    let w = world(Profile::standard(), 4);
    w.stand_in.with(|d| d.max_set = 2);
    let cancel = Rc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let server = Rewrite {
        inner: w.stand_in.clone(),
        with: Rc::new(move |asked: &Value, body: Value| {
            if asked.pointer("/methodCalls/0/0") == Some(&json!("Email/set")) {
                flag.store(true, Ordering::Relaxed);
            }
            body
        }),
    };
    let pieces = run_over(server, &request(changes, &[]), &cancel);
    assert_eq!(outcomes(&pieces).len(), 2);
    assert!(last(&pieces).stopped && !last(&pieces).whole);
    assert_eq!(sets(&w.stand_in), [(2, 0)]);
}

#[test]
fn an_answer_that_was_lost_is_no_outcome_and_the_changes_are_worked_out_again() {
    // Marks: made, though nobody was told; made again, which changes nothing. A mark that waits changes nothing a window
    // says, so the run is whole.
    let w = world(Profile::standard(), 3);
    let ids = w.mails.clone();
    w.stand_in.with(|d| d.lost_answers = 1);
    let changes = vec![read(&ids[0], true), read(&ids[2], true)];
    let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
    assert!(only_outcomes(&pieces).is_empty());
    let end = last(&pieces);
    assert!(end.whole && end.errors.len() == 1 && end.errors[0].starts_with("Couldn't send 2 changes to the server: "), "{end:?}");
    assert!(!end.errors[0].contains("example.org"), "{}", end.errors[0]);
    assert_eq!(w.state_of(1).unwrap().0, ["$seen"], "it was done");
    assert!(pieces.iter().any(|piece| !piece.pages.is_empty()), "and the mailboxes are read");
    let again = run(&w.stand_in, &request(changes, &[]));
    assert_eq!(only_outcomes(&again), [done(""), done("")]);
    assert!(last(&again).whole && last(&again).errors.is_empty());

    // A move: made as well, and then no window of the mailbox it left lists the message, so the run is not whole;
    // made again, which says the same.
    let w = world(Profile::standard(), 3);
    let ids = w.mails.clone();
    w.stand_in.with(|d| d.lost_answers = 1);
    let changes = vec![moved(&ids[1], &w.inbox, &w.archive)];
    let pieces = run(&w.stand_in, &request(changes.clone(), &[&w.archive]));
    assert!(only_outcomes(&pieces).is_empty() && !last(&pieces).whole);
    assert_eq!(w.state_of(2).unwrap().1, ["Archive"]);
    let again = run(&w.stand_in, &request(changes, &[&w.archive]));
    assert_eq!(only_outcomes(&again), [done(&ids[1])]);
    assert!(last(&again).whole);

    // A server that names mail by place: the move was made, so the old id names nothing; the server does not look, and
    // says it was made again, and the lookup of the old id finds nothing: the message has no id until a window shows it.
    let w = world(Profile::by_place(), 3);
    let ids = w.mails.clone();
    w.stand_in.with(|d| d.lost_answers = 1);
    let changes = vec![moved(&ids[0], &w.inbox, &w.archive)];
    let pieces = run(&w.stand_in, &request(changes.clone(), &[&w.archive]));
    assert!(only_outcomes(&pieces).is_empty() && !last(&pieces).whole);
    assert_eq!(w.state_of(1).unwrap().1, ["Archive"]);
    let again = run(&w.stand_in, &request(changes, &[&w.archive]));
    assert_eq!(only_outcomes(&again), [done("")]);
}

#[test]
fn ids_that_cannot_be_a_servers_are_refused_without_asking_and_what_needs_no_server_is_done() {
    let w = world(Profile::standard(), 2);
    let ids = w.mails.clone();
    let changes = vec![
        read("a b", true),
        read("", true),
        marks(&ids[0], None, None, None, None),
        moved(&ids[0], "x y", &w.archive),
        moved(&ids[0], &w.inbox, "../trash"),
        moved(&ids[0], &w.inbox, &w.inbox),
        destroy("a/b"),
        read(&"x".repeat(513), true),
    ];
    let pieces = run(&w.stand_in, &request(changes, &[]));
    assert_eq!(
        only_outcomes(&pieces),
        [
            refused("invalidId"),
            refused("invalidId"),
            done(""),
            refused("invalidId"),
            refused("invalidId"),
            done(&ids[0]),
            refused("invalidId"),
            refused("invalidId")
        ]
    );
    assert!(sets(&w.stand_in).is_empty(), "nothing was asked of the server");
    assert!(last(&pieces).whole);

    // The same email destroyed twice is destroyed once, and both changes are done; two changes of marks are one patch,
    // the later of a mark winning.
    let pieces = run(&w.stand_in, &request(vec![destroy(&ids[0]), read(&ids[1], true), destroy(&ids[0]), read(&ids[1], false)], &[]));
    assert_eq!(only_outcomes(&pieces), [done(""), done(""), done(""), done("")]);
    assert_eq!(sets(&w.stand_in), [(1, 0), (0, 1)]);
    assert!(w.state_of(2).unwrap().0.is_empty() && w.state_of(1).is_none());
}

#[test]
fn nothing_is_asked_and_nothing_is_handed_on_when_there_are_no_changes() {
    let w = world(Profile::standard(), 2);
    let pieces = run(&w.stand_in, &request(Vec::new(), &[]));
    assert!(pieces.iter().all(|piece| piece.changes.is_empty()));
    assert!(sets(&w.stand_in).is_empty() && !w.stand_in.calls().iter().any(|call| call == "Email/set"));
    assert!(last(&pieces).whole);
}

#[test]
fn the_changes_go_up_before_the_mailboxes_are_read_and_what_is_read_is_the_server_after_them() {
    let w = world(Profile::standard(), 3);
    let ids = w.mails.clone();
    let pieces = run(&w.stand_in, &request(vec![destroy(&ids[0]), moved(&ids[1], &w.inbox, &w.trash), read(&ids[2], true)], &[]));
    let calls = w.stand_in.calls();
    assert_eq!(&calls[..4], ["Email/set", "Email/set", "Email/get", "Mailbox/get"], "{calls:?}");
    // The Inbox, read after, has the one email, read; and it is the whole of it.
    let inbox = pieces.iter().flat_map(|piece| piece.pages.iter()).find(|page| page.folder == w.inbox).unwrap();
    assert!(inbox.afresh);
    assert_eq!(inbox.messages.iter().map(|m| (m.message_id.clone(), m.server.read)).collect::<Vec<_>>(), [("m3@example.org".to_string(), true)]);
    // The counts of the mailbox list were made after the changes: the Trash holds the one.
    let listing_counts: BTreeMap<String, u32> = w
        .stand_in
        .with(|d| d.mailboxes.iter().map(|m| (m.name.clone(), d.emails.iter().filter(|e| e.mailboxes.contains(&m.id)).count() as u32)).collect());
    assert_eq!((listing_counts["Inbox"], listing_counts["Trash"]), (1, 1));
    assert!(last(&pieces).whole);
}

#[test]
fn what_a_hostile_server_answers_to_a_set_is_only_looked_at_for_the_ids_that_were_sent() {
    let w = world(Profile::standard(), 4);
    let ids = w.mails.clone();
    let wide: Vec<Value> = (0..100_000).map(|n| json!(format!("E{n}"))).collect();
    let cases: Vec<(&str, Value, Vec<JmapOutcome>)> = vec![
        // Not a map, not a list, not there.
        ("shapes", json!({"updated": [1, 2], "notUpdated": "x", "destroyed": {"a": 1}, "notDestroyed": 5}), vec![]),
        ("nothing", json!({}), vec![]),
        ("null", Value::Null, vec![]),
        // Ids nobody sent; the ones sent are looked up.
        ("others", json!({"updated": {"E99": null, &ids[1]: null}, "destroyed": ["E98", &ids[3]]}), vec![done(""), done("")]),
        // Updated and not: the refusal stands.
        ("both", json!({"updated": {&ids[0]: null}, "notUpdated": {&ids[0]: {"type": "forbidden"}}}), vec![refused("forbidden")]),
        // A new id that is no id is none; a new id for a mark is of no account.
        ("ids", json!({"updated": {&ids[0]: {"id": "not an id!"}, &ids[1]: {"id": "E7"}}}), vec![done(""), done("")]),
        // A great many listed.
        ("wide", json!({"destroyed": wide, "updated": {&ids[0]: null}}), vec![done("")]),
    ];
    for (what, result, expected) in cases {
        let server = saying(&w.stand_in, {
            let result = result.clone();
            move |_, _| Say::Result(result.clone())
        });
        let changes = match what {
            "ids" => vec![read(&ids[0], true), read(&ids[1], true)],
            "others" => vec![read(&ids[1], true), destroy(&ids[3])],
            "wide" => vec![read(&ids[0], true)],
            "both" => vec![read(&ids[0], true)],
            _ => vec![read(&ids[0], true), read(&ids[1], true)],
        };
        let pieces = run_over(server, &request(changes, &[]), &AtomicBool::new(false));
        assert_eq!(only_outcomes(&pieces), expected, "{what}");
        // Marks that no answer was heard of change nothing a window says: the run is whole.
        let end = last(&pieces);
        assert!(end.whole, "{what}: {end:?}");
    }
    // A move that the server answers with the old id's presence only: found by the second call.
    let moved_answer = saying(&w.stand_in, {
        let id = ids[2].clone();
        move |_, _| Say::Result(json!({"updated": {&id: {"id": "Enew-id"}}}))
    });
    let pieces = run_over(moved_answer, &request(vec![moved(&ids[2], &w.inbox, &w.archive)], &[]), &AtomicBool::new(false));
    assert_eq!(only_outcomes(&pieces), [done("Enew-id")], "the id the answer names");
}

fn small_ids() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => proptest::sample::select(vec!["Eid1", "Eid2", "Eid3", "Eid4"]).prop_map(str::to_string),
        1 => "[ -~]{0,6}",
    ]
}

fn any_change() -> impl Strategy<Value = JmapChange> {
    let flag = proptest::option::of(any::<bool>());
    prop_oneof![
        (small_ids(), flag.clone(), flag.clone(), flag.clone(), flag).prop_map(|(id, read, flagged, answered, forwarded)| JmapChange::Keywords {
            message: 1,
            id,
            read,
            flagged,
            answered,
            forwarded
        }),
        (small_ids(), small_ids(), small_ids()).prop_map(|(id, from, to)| JmapChange::Move { message: 1, id, from, to }),
        small_ids().prop_map(|id| JmapChange::Destroy { id }),
    ]
}

proptest! {
    /// Whatever the changes and whatever the server answers, a run ends with a last piece, and answers only the changes
    /// it was asked, in order and once each.
    #[test]
    fn whatever_is_asked_and_answered_the_run_ends_and_answers_only_what_was_asked(
        changes in proptest::collection::vec(any_change(), 0..12),
        answer in crate::tests::any_json(),
        hostile in any::<bool>(),
        by_place in any::<bool>(),
    ) {
        let w = world(if by_place { Profile::by_place() } else { Profile::standard() }, 3);
        let request = request(changes, &[]);
        let server = if hostile { saying(&w.stand_in, move |_, _| Say::Result(answer.clone())) } else { saying(&w.stand_in, |_, result| Say::Result(result)) };
        let pieces = run_over(server, &request, &AtomicBool::new(false));
        let answered = outcomes(&pieces).len();
        prop_assert!(answered <= request.changes.len());
        prop_assert!(!last(&pieces).partial);
        // A server that answers as it should answers every change, and the run is then whole.
        if !hostile {
            prop_assert_eq!(answered, request.changes.len());
            prop_assert!(last(&pieces).whole, "{:?}", last(&pieces));
        }
    }
}

proptest! {
    /// Against the in-process server, whatever the changes of distinct emails, the server ends as a model of the
    /// changes says, whichever kind of server it is.
    #[test]
    fn the_server_ends_as_the_changes_say(
        plan in proptest::collection::vec((0usize..4, proptest::option::of(any::<bool>()), proptest::option::of(any::<bool>()), 0u8..4), 1..5),
        by_place in any::<bool>(),
        max_set in 1usize..6,
    ) {
        let w = world(if by_place { Profile::by_place() } else { Profile::standard() }, 4);
        w.stand_in.with(|d| d.max_set = max_set);
        let ids = w.mails.clone();
        let mut changes = Vec::new();
        let mut model: BTreeMap<usize, Option<(BTreeSet<String>, String)>> = (0..4).map(|k| (k, Some((BTreeSet::new(), "Inbox".to_string())))).collect();
        let mut used = BTreeSet::new();
        for (k, read_to, flagged_to, action) in plan {
            if !used.insert(k) { continue; }
            let id = &ids[k];
            match action {
                0 => {
                    changes.push(marks(id, read_to, flagged_to, None, None));
                    if let Some(Some((keywords, _))) = model.get_mut(&k) {
                        for (name, on) in [("$seen", read_to), ("$flagged", flagged_to)] {
                            match on { Some(true) => { keywords.insert(name.to_string()); } Some(false) => { keywords.remove(name); } None => {} }
                        }
                    }
                }
                1 => {
                    changes.push(moved(id, &w.inbox, &w.archive));
                    if let Some(Some((_, place))) = model.get_mut(&k) { *place = "Archive".into(); }
                }
                2 => {
                    changes.push(destroy(id));
                    model.insert(k, None);
                }
                _ => {
                    changes.push(marks(id, read_to, None, None, None));
                    changes.push(moved(id, &w.inbox, &w.trash));
                    if let Some(Some((keywords, place))) = model.get_mut(&k) {
                        match read_to { Some(true) => { keywords.insert("$seen".to_string()); } Some(false) => { keywords.remove("$seen"); } None => {} }
                        *place = "Trash".into();
                    }
                }
            }
        }
        let pieces = run(&w.stand_in, &request(changes.clone(), &[]));
        for (k, expected) in &model {
            let found = w.state_of(k + 1).map(|(keywords, places)| (keywords.into_iter().collect::<BTreeSet<_>>(), places.join(",")));
            prop_assert_eq!(&found, expected, "email {}", k + 1);
        }
        prop_assert_eq!(outcomes(&pieces).len(), changes.len());
        prop_assert!(last(&pieces).whole);
        // Each call held at most `max_set` objects.
        prop_assert!(sets(&w.stand_in).iter().all(|(u, d)| u + d <= max_set));
    }
}
