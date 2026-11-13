// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! « Trier avec Nora » with a host that keeps every call for the test to answer: what the server's answers say, read
//! warily; where nothing of it is offered; and each turn of the flow (no plan yet, `needs_plan`, a plan made in the
//! background, the tidying followed until it is done, an undo called again while some are left, the rhythm after it), the
//! person who only reads the mailbox, a site whose plan has no background assistants, and the server's refusals (403, 417)
//! said in its own words. A hosting page's mailbox of invented people at `example.org`.

use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use super::super::testing::{Calls, ME, page};
use super::super::{NoraAnswer, NoraCall, SORTING_METHODS};
use super::*;
use crate::MailApp;

/// The page's mailbox, as the server names it (the page says no other name).
const BOX: &str = ME;

/// A hosting page's tab (nothing kept here) with its Inbox in view, and the host's calls.
fn hosted() -> (MailApp, Calls) {
    let (mut app, calls, _) = page(true);
    app.session.keeps_local = false;
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    assert_eq!(box_in_view(&app, true).as_deref(), Some(BOX));
    (app, calls)
}

/// Answer the latest call of `method` that has no answer yet, with `body` as the server writes it.
fn reply(calls: &Calls, method: &str, code: u16, body: Value) {
    let mut kept = calls.0.borrow_mut();
    let (call, to): &mut (NoraCall, Option<Sender<NoraAnswer>>) =
        kept.iter_mut().rev().find(|(call, to)| call.method == method && to.is_some()).unwrap_or_else(|| panic!("no call of {method}"));
    to.take().unwrap().send(NoraAnswer { id: call.id, status: code, body: body.to_string() }).unwrap();
}

/// The calls of `method` so far, by their arguments.
fn made(calls: &Calls, method: &str) -> Vec<Value> {
    calls.0.borrow().iter().filter(|(call, _)| call.method == method).map(|(call, _)| call.args.clone()).collect()
}

/// Whether a call of `method` waits for an answer.
fn waiting(calls: &Calls, method: &str) -> bool {
    calls.0.borrow().iter().any(|(call, to)| call.method == method && to.is_some())
}

fn ok(message: Value) -> Value {
    json!({"message": message})
}

/// What `state` says of the box: Nora on or off, the plan's state (""/"proposed"/"validated"), whether the caller may change it.
pub(crate) fn state(on: bool, plan: &str, may_set: bool) -> Value {
    json!({
        "account": BOX, "address": BOX, "available": true, "unavailable": "", "streams": true,
        "modes": [
            {"code": "clients", "label": "One folder per client"},
            {"code": "activity", "label": "By activity"},
            {"code": "streams", "label": "Notifications and newsletters only"}
        ],
        "intervals": [5, 15, 60], "secretary": true, "may_set": may_set, "on": on,
        "reading": {"enabled": true, "listened": true, "interval": 15, "folders": 4, "last_run_at": "2026-10-14 15:45:00", "next_at": "2026-10-14 16:00:00",
                    "next_said": "16:00", "failures": 0, "prepare_documents": false, "prepare_replies": false},
        "mine": null,
        "plan": if plan.is_empty() { Value::Null } else { json!({"status": plan, "mode": "clients", "folders": 4, "keep_unread_hours": 24, "proposed_at": "", "validated_at": "", "filed": 0}) },
        "tidy": {"status": "", "since": "", "run": "", "filed": 0, "left": 0, "seconds": 0, "waiting": 12, "asked_by": "", "asked_at": "", "done_at": "", "error": "", "days": [30, 90, 365, 0], "max": 5000},
        "asks": 1,
        "last_run": null
    })
}

/// A plan by client, as `plan` (and `propose`, `validate`) says it.
pub(crate) fn plan(status: &str) -> Value {
    json!({
        "account": BOX, "address": BOX, "status": status, "on": false, "mode": "clients", "wishes": "", "keep_unread_hours": 24, "may_set": true,
        "since_said": if status == "Validated" { "14.10.2026" } else { "" }, "proposed_at": "2026-10-14 15:50:00", "validated_by": "", "validated_at": null,
        "folders": [
            {"folder": "Clients", "holds": "One folder per client", "in_box": false, "count": 47, "examples": ["Devis 118"], "takes": "clients"},
            {"folder": "Notifications", "holds": "Tools", "in_box": false, "count": 8, "examples": [], "takes": "notifications"},
            {"folder": "Bulletins d'information", "holds": "Newsletters", "in_box": false, "count": 6, "examples": [], "takes": "newsletters"},
            {"folder": "Factures reçues", "holds": "Invoices", "in_box": true, "count": 18, "examples": [], "takes": "invoices"}
        ],
        "clients": {"shown": [{"name": "Atelier Exemple", "count": 33}, {"name": "Garage Exemple", "count": 14}], "more": 3, "more_count": 9, "total": 5, "mails": 56},
        "mails": 91, "tried": 30, "left": 6, "read": 91, "needed": 20, "filed": 0
    })
}

pub(crate) fn survey() -> Value {
    json!({"days": 30, "mails": 91, "sorted": 85, "kinds": {"notification": 8, "newsletter": 6, "invoice": 18, "question": 12, "unsorted": 6},
           "clients": 2, "client_mails": 47, "in_inbox": 35, "read_since": "2026-09-01 07:00:00", "catch_up": "done", "secretary": true})
}

/// The tidying, as `state.tidy` says it.
fn tidy(status: &str, run: &str, filed: u64) -> Value {
    json!({"status": status, "since": "", "run": run, "filed": filed, "left": 0, "seconds": 4.3, "waiting": 0, "asked_by": "Ada", "asked_at": "", "done_at": "", "error": "", "days": [30, 90, 365, 0], "max": 5000})
}

/// A journal whose newest run is `run`, a tidying that filed 23 mails.
pub(crate) fn journal(run: &str) -> Value {
    json!({
        "runs": [{
            "run": run, "kind": "tidy", "status": "done", "started_at": "2026-10-14 15:55:00", "ended_at": "2026-10-14 15:55:04", "seconds": 4.3,
            "filed": 23, "left": 6, "failed": 0, "undone": 0, "still_filed": 23,
            "folders": [
                {"folder": "Factures reçues", "parent": "", "leaf": "Factures reçues", "count": 9, "takes": "invoices"},
                {"folder": "Notifications", "parent": "", "leaf": "Notifications", "count": 7, "takes": "notifications"},
                {"folder": "Clients/Atelier Exemple", "parent": "Clients", "leaf": "Atelier Exemple", "count": 5, "takes": "clients"},
                {"folder": "Bulletins d'information", "parent": "", "leaf": "Bulletins d'information", "count": 2, "takes": "newsletters"}
            ],
            "kinds": {"invoices": 9, "notifications": 7, "clients": 5, "newsletters": 2},
            "can_undo": true
        }],
        "learned": [{"mail": "M1", "folder": "Notifications", "leaf": "Notifications", "from": "Outil Exemple", "subject": "Alerte", "at": "2026-10-14 15:30:00"}],
        "may_set": true
    })
}

fn asks() -> Value {
    json!({"asks": [{"name": "ASK-0001", "kind": "Family", "topic": "Task", "title": "Payment notices from your bank: shall I make a task of each?",
                     "context": "3 this month", "choices": [{"code": "task", "label": "Yes, a task"}, {"code": "none", "label": "No, file them"}], "box": BOX}],
           "retained": 2})
}

/// Frappe's refusal (`frappe.throw`): `exc_type` and the person's sentence in `_server_messages`.
fn thrown(exc: &str, words: &str) -> Value {
    let shown = json!({"message": words}).to_string();
    json!({"exc_type": exc, "_server_messages": json!([shown]).to_string()})
}

/// The window reads its answers.
fn take(app: &mut MailApp) {
    poll(app);
}

/// The window with the state of the box in view said: Nora on or off, the plan's state, the caller's rights.
fn with_state(on: bool, plan_status: &str, may_set: bool) -> (MailApp, Calls) {
    let (mut app, calls) = hosted();
    take(&mut app);
    assert_eq!(made(&calls, STATE), vec![json!({"account": BOX})], "the state of the box in view is asked");
    reply(&calls, STATE, 200, ok(state(on, plan_status, may_set)));
    take(&mut app);
    (app, calls)
}

#[test]
fn nothing_is_asked_in_a_window_that_keeps_its_mail_nor_through_a_host_that_doesnt_carry_it() {
    // The program that keeps its mail on the computer.
    let (mut app, calls, _) = page(true);
    app.execute("folder.select", json!({"kind": "inbox"})).unwrap();
    take(&mut app);
    assert!(made(&calls, STATE).is_empty());
    assert!(!offered(&app) && box_in_view(&app, true).is_none());
    // A hosting page whose host doesn't say it carries these calls (Neodesk before its list has them).
    app.session.keeps_local = false;
    app.services.nora.as_mut().unwrap().sorting = false;
    take(&mut app);
    assert!(made(&calls, STATE).is_empty());
    // Where it does: the state, once; not again before a minute.
    app.services.nora.as_mut().unwrap().sorting = true;
    take(&mut app);
    take(&mut app);
    assert_eq!(made(&calls, STATE).len(), 1);
}

#[test]
fn the_state_is_read_field_by_field() {
    let read = read_state(&state(true, "validated", true));
    assert!(read.available && read.on && read.may_set && read.secretary && read.streams && read.validated());
    assert_eq!(read.modes.iter().map(|m| m.code.as_str()).collect::<Vec<_>>(), ["clients", "activity", "streams"]);
    assert_eq!(read.intervals, [5, 15, 60]);
    assert_eq!((read.interval, read.next_said.as_str(), read.asks), (15, "16:00", 1));
    assert_eq!(read.tidy.as_ref().map(|t| t.status.as_str()), Some(""));
    // A box without secretary: nothing more.
    let bare = read_state(&json!({"available": true, "secretary": false, "may_set": true, "on": false, "reading": null, "plan": null, "tidy": null}));
    assert!(bare.available && !bare.secretary && bare.may_set && bare.plan_status.is_empty() && bare.tidy.is_none() && bare.interval == 0);
    // What isn't what it should be is nothing: a mode that is no code, an interval out of range, numbers as text, a list too long.
    let odd = read_state(&json!({
        "available": "1", "may_set": 0, "on": "true",
        "modes": [{"code": "Par projet!", "label": "Par projet"}, {"code": "clients", "label": ""}, {"code": "streams", "label": "Streams"}, 7],
        "intervals": [0, 15, 99999, "60", -5],
        "reading": {"interval": "15", "next_said": "10:45\u{202E}"},
        "plan": {"status": "Validated"},
        "tidy": {"status": "Running", "filed": 12.0, "run": "x".repeat(500)},
        "last_run": {"run": "R1", "kind": "Tidy", "status": "Partly undone", "filed": "5"},
        "asks": -3
    }));
    assert!(odd.available && !odd.may_set && odd.on);
    assert_eq!(odd.modes, vec![Mode { code: "streams".into(), label: "Streams".into() }]);
    assert_eq!(odd.intervals, [15]);
    assert_eq!((odd.interval, odd.next_said.as_str()), (15, "10:45"), "no character that turns text round");
    assert!(odd.validated() && odd.tidying());
    assert_eq!(odd.tidy.as_ref().map(|t| (t.filed, t.run.len())), Some((12, 0)), "a name too long is no name");
    let last = odd.last_run.unwrap();
    assert_eq!((last.kind.as_str(), last.status.as_str(), last.filed), ("tidy", "partly_undone", 5));
    assert_eq!(odd.asks, 0);
}

#[test]
fn the_plan_the_survey_the_journal_and_the_questions_are_read() {
    let plan = read_plan(&plan("Proposed"));
    assert_eq!(plan.status, "proposed");
    assert_eq!(
        plan.folders.iter().map(|f| f.folder.as_str()).collect::<Vec<_>>(),
        ["Clients", "Notifications", "Bulletins d'information", "Factures reçues"]
    );
    assert_eq!(plan.folders[0].takes, "clients");
    let clients = plan.clients.unwrap();
    assert_eq!(clients.shown, vec![("Atelier Exemple".to_string(), 33), ("Garage Exemple".to_string(), 14)]);
    assert_eq!((clients.more, clients.more_count, plan.keep_unread_hours), (3, 9, 24));
    assert!(read_plan(&json!({"queued": true, "mode": "activity"})).queued);
    let survey = read_survey(&survey());
    assert_eq!((survey.mails, survey.clients, survey.kind("notification"), survey.kind("newsletter"), survey.kind("order")), (91, 2, 8, 6, 0));
    let journal = read_journal(&journal("NFR-1"));
    let run = &journal.runs[0];
    assert_eq!((run.run.as_str(), run.kind.as_str(), run.filed, run.can_undo), ("NFR-1", "tidy", 23, true));
    assert_eq!(run.folders[2].said(), "Clients › Atelier Exemple");
    assert_eq!(run.folders[0].said(), "Factures reçues");
    assert_eq!(journal.learned[0].leaf, "Notifications");
    let (asked, retained) = read_asks(&asks());
    assert_eq!(asked[0].choices, vec![("task".to_string(), "Yes, a task".to_string()), ("none".to_string(), "No, file them".to_string())]);
    assert_eq!((asked[0].mailbox.as_str(), retained), (BOX, 2));
    // A question without a name or a title, a choice without a label: left out.
    assert!(read_asks(&json!({"asks": [{"name": "", "title": "t"}, {"name": "A", "title": ""}]})).0.is_empty());
    let undone = read_undone(&json!({"undone": 500, "moved": 2, "gone": 1, "failed": 0, "remaining": 23}));
    assert_eq!(undone, Undone { undone: 500, moved: 2, gone: 1, failed: 0, remaining: 23 });
}

#[test]
fn without_a_validated_plan_the_switch_opens_the_card_and_the_card_turns_nora_on_then_asks_for_a_plan() {
    let (mut app, calls) = with_state(false, "", true);
    act(&mut app, Intent::Switch { account: BOX.into(), on: true });
    assert!(made(&calls, SWITCH).is_empty(), "nothing is turned on before the card");
    let card = app.nora.sorting.card.clone().unwrap();
    assert_eq!((card.mode.as_str(), card.interval, card.prepare), ("clients", 15, false));
    app.nora.sorting.card.as_mut().unwrap().interval = 60;
    app.nora.sorting.card.as_mut().unwrap().prepare = true;
    act(&mut app, Intent::Activate);
    assert_eq!(made(&calls, SWITCH), vec![json!({"account": BOX, "on": 1, "interval": 60, "prepare_documents": 1, "prepare_replies": 1})]);
    // The server turns the reading on and says it needs a plan: the flow opens on one, made the way the card chose.
    let mut answered = state(true, "", true);
    answered["needs_plan"] = json!(true);
    reply(&calls, SWITCH, 200, ok(answered));
    take(&mut app);
    assert!(app.nora.sorting.card.is_none());
    assert!(app.nora.assist.open, "her pane is open on the flow");
    let flow = app.nora.sorting.flow.as_ref().unwrap();
    assert!(flow.on_intent);
    assert_eq!(flow.stage, Stage::Proposing);
    assert_eq!(made(&calls, PROPOSE), vec![json!({"account": BOX, "mode": "clients", "keep_unread": 1})]);
    reply(&calls, PROPOSE, 200, ok(plan("Proposed")));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Plan);
    // « Ranger maintenant »: the plan validated as proposed, the new mail filed from now on (she was turned on), the inbox tidied.
    act(&mut app, Intent::FileNow);
    assert_eq!(made(&calls, VALIDATE), vec![json!({"account": BOX, "folders": null, "file_mails": 1, "tidy_days": 30})]);
}

#[test]
fn with_a_validated_plan_the_switch_turns_nora_on_and_off_at_once() {
    let (mut app, calls) = with_state(false, "validated", true);
    act(&mut app, Intent::Switch { account: BOX.into(), on: true });
    assert!(app.nora.sorting.card.is_none());
    assert_eq!(made(&calls, SWITCH), vec![json!({"account": BOX, "on": 1})]);
    reply(&calls, SWITCH, 200, ok(state(true, "validated", true)));
    take(&mut app);
    assert!(app.nora.sorting.available(BOX).unwrap().on);
    act(&mut app, Intent::Switch { account: BOX.into(), on: false });
    assert_eq!(made(&calls, SWITCH).last(), Some(&json!({"account": BOX, "on": 0})));
    // The plan was no longer validated meanwhile: the server says so, and the card asks how to file.
    let (mut app, calls) = with_state(false, "validated", true);
    act(&mut app, Intent::Switch { account: BOX.into(), on: true });
    let mut answered = state(false, "proposed", true);
    answered["needs_plan"] = json!(true);
    reply(&calls, SWITCH, 200, ok(answered));
    take(&mut app);
    assert!(app.nora.sorting.card.is_some());
}

#[test]
fn the_flow_reads_what_nora_read_offers_the_servers_ways_and_a_plan_by_activity_is_read_again_until_proposed() {
    let (mut app, calls) = with_state(false, "", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: false });
    for method in [SURVEY, JOURNAL] {
        assert!(waiting(&calls, method), "{method}");
    }
    assert_eq!(made(&calls, SURVEY), vec![json!({"account": BOX, "days": 30})]);
    reply(&calls, STATE, 200, ok(state(false, "", true)));
    reply(&calls, SURVEY, 200, ok(survey()));
    reply(&calls, JOURNAL, 200, ok(json!({"runs": [], "learned": [], "may_set": true})));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Choose);
    assert!(made(&calls, PLAN).is_empty(), "no plan yet: none is asked");
    let activity = app.nora.sorting.available(BOX).unwrap().modes[1].clone();
    act(&mut app, Intent::Choose(activity));
    reply(&calls, PROPOSE, 200, ok(json!({"queued": true, "mode": "activity"})));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Proposing);
    // Read again a few seconds later, until the server says it is proposed.
    app.nora.sorting.flow.as_mut().unwrap().next = 0.0;
    take(&mut app);
    reply(&calls, PLAN, 200, ok(json!({"status": "", "folders": [], "may_set": true})));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Proposing);
    app.nora.sorting.flow.as_mut().unwrap().next = 0.0;
    take(&mut app);
    reply(&calls, PLAN, 200, ok(plan("Proposed")));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Plan);
    // Given up after two minutes, with why.
    act(&mut app, Intent::NewPlan);
    let mode = app.nora.sorting.available(BOX).unwrap().modes[1].clone();
    act(&mut app, Intent::Choose(mode));
    reply(&calls, PROPOSE, 200, ok(json!({"queued": true})));
    take(&mut app);
    app.nora.sorting.flow.as_mut().unwrap().since -= PLAN_GIVE_UP_MS + 1.0;
    take(&mut app);
    let flow = app.nora.sorting.flow.as_ref().unwrap();
    assert_eq!(flow.stage, Stage::Choose);
    assert!(flow.error.as_deref().is_some_and(|e| e.contains("too long")), "{:?}", flow.error);
}

#[test]
fn file_now_validates_and_follows_the_tidying_until_it_is_done_then_reads_the_journal() {
    let (mut app, calls) = with_state(false, "proposed", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: false });
    reply(&calls, STATE, 200, ok(state(false, "proposed", true)));
    take(&mut app);
    reply(&calls, PLAN, 200, ok(plan("Proposed")));
    reply(&calls, SURVEY, 200, ok(survey()));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Plan);
    act(&mut app, Intent::FileNow);
    assert_eq!(made(&calls, VALIDATE), vec![json!({"account": BOX, "folders": null, "file_mails": 0, "tidy_days": 30})]);
    let mut validated = plan("Validated");
    validated["tidy"] = tidy("running", "NFR-7", 0);
    reply(&calls, VALIDATE, 200, ok(validated));
    take(&mut app);
    let flow = app.nora.sorting.flow.as_ref().unwrap();
    assert_eq!((flow.stage, flow.tidy_run.as_str()), (Stage::Tidying, "NFR-7"));
    // Followed by the state, every two seconds, until it is done.
    let mut running = state(false, "validated", true);
    running["tidy"] = tidy("running", "NFR-7", 12);
    reply(&calls, STATE, 200, ok(running.clone()));
    take(&mut app);
    app.nora.sorting.flow.as_mut().unwrap().next = 0.0;
    take(&mut app);
    reply(&calls, STATE, 200, ok(running));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Tidying);
    assert_eq!(app.nora.sorting.available(BOX).unwrap().tidy.as_ref().unwrap().filed, 12);
    app.nora.sorting.flow.as_mut().unwrap().next = 0.0;
    take(&mut app);
    let mut done = state(false, "validated", true);
    done["tidy"] = tidy("done", "NFR-7", 23);
    reply(&calls, STATE, 200, ok(done));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Done);
    assert!(waiting(&calls, JOURNAL));
    reply(&calls, JOURNAL, 200, ok(journal("NFR-7")));
    take(&mut app);
    let flow = app.nora.sorting.flow.as_ref().unwrap();
    assert_eq!(flow.journal.as_ref().unwrap().runs[0].run, "NFR-7");
}

#[test]
fn a_validated_plan_tidies_without_validating_again_and_a_plan_changed_goes_as_the_person_left_it() {
    let (mut app, calls) = with_state(false, "validated", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: false });
    reply(&calls, STATE, 200, ok(state(false, "validated", true)));
    take(&mut app);
    reply(&calls, PLAN, 200, ok(plan("Validated")));
    take(&mut app);
    act(&mut app, Intent::FileNow);
    assert!(made(&calls, VALIDATE).is_empty());
    assert_eq!(made(&calls, TIDY), vec![json!({"account": BOX, "days": 30})]);
    reply(&calls, TIDY, 200, ok(tidy("running", "NFR-8", 0)));
    take(&mut app);
    assert_eq!(app.nora.sorting.flow.as_ref().unwrap().stage, Stage::Tidying);
    // « Modifier »: a folder renamed, another left out.
    app.nora.sorting.flow.as_mut().unwrap().enter(Stage::Plan, 0.0);
    act(&mut app, Intent::Edit);
    let rows = app.nora.sorting.flow.as_mut().unwrap().editing.as_mut().unwrap();
    rows[1].folder = "  Alertes ".into();
    rows[2].kept = false;
    act(&mut app, Intent::FileNow);
    let sent = made(&calls, VALIDATE);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0]["folders"],
        json!([
            {"folder": "Clients", "holds": "One folder per client", "takes": "clients"},
            {"folder": "Alertes", "holds": "Tools", "takes": "notifications"},
            {"folder": "Factures reçues", "holds": "Invoices", "takes": "invoices"}
        ])
    );
}

#[test]
fn an_undo_is_called_again_while_the_server_says_some_are_left() {
    let (mut app, calls) = with_state(true, "validated", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: true });
    reply(&calls, JOURNAL, 200, ok(journal("NFR-9")));
    take(&mut app);
    act(&mut app, Intent::Undo("NFR-9".into()));
    assert_eq!(made(&calls, UNDO), vec![json!({"account": BOX, "run": "NFR-9"})]);
    reply(&calls, UNDO, 200, ok(json!({"undone": 500, "moved": 0, "gone": 0, "failed": 0, "remaining": 23})));
    take(&mut app);
    assert_eq!(made(&calls, UNDO).len(), 2, "called again at once");
    reply(&calls, UNDO, 200, ok(json!({"undone": 21, "moved": 2, "gone": 0, "failed": 0, "remaining": 0})));
    take(&mut app);
    let said = app.nora.sorting.flow.as_ref().unwrap().said.clone().unwrap();
    assert!(said.contains("21 messages are back") && said.contains("2 messages moved by someone"), "{said}");
    assert!(waiting(&calls, JOURNAL) && waiting(&calls, STATE), "the journal and the state are read again");
}

#[test]
fn the_person_who_only_reads_the_mailbox_changes_nothing() {
    let (mut app, calls) = with_state(true, "validated", false);
    act(&mut app, Intent::Switch { account: BOX.into(), on: false });
    assert!(made(&calls, SWITCH).is_empty() && app.nora.sorting.card.is_none());
    let home = super::super::sorting_view::home_of(&app).unwrap();
    assert!(!home.may_set && home.on);
}

#[test]
fn a_site_whose_plan_has_no_background_assistants_shows_nothing() {
    let (mut app, calls) = hosted();
    take(&mut app);
    let mut off = state(false, "", true);
    off["available"] = json!(false);
    off["unavailable"] = json!("plan");
    reply(&calls, STATE, 200, ok(off));
    take(&mut app);
    assert!(app.nora.sorting.available(BOX).is_none());
    assert!(super::super::sorting_view::home_of(&app).is_none());
    act(&mut app, Intent::Switch { account: BOX.into(), on: true });
    assert!(made(&calls, SWITCH).is_empty() && app.nora.sorting.card.is_none());
}

#[test]
fn a_refusal_is_said_in_the_servers_words() {
    // 403: the person doesn't read the box. Nothing of it is shown.
    let (mut app, calls) = hosted();
    take(&mut app);
    reply(&calls, STATE, 403, thrown("PermissionError", "Vous ne lisez pas cette boîte dans NeoMail."));
    take(&mut app);
    assert!(app.nora.sorting.available(BOX).is_none());
    // 417: a plan that can't be validated; the flow says why, as the server wrote it.
    let (mut app, calls) = with_state(false, "proposed", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: false });
    reply(&calls, STATE, 200, ok(state(false, "proposed", true)));
    take(&mut app);
    reply(&calls, PLAN, 200, ok(plan("Proposed")));
    take(&mut app);
    act(&mut app, Intent::FileNow);
    reply(&calls, VALIDATE, 417, thrown("ValidationError", "Nora range cette boîte en ce moment : réessayez dans une minute."));
    take(&mut app);
    let flow = app.nora.sorting.flow.as_ref().unwrap();
    assert_eq!(flow.stage, Stage::Plan);
    assert!(
        flow.error.as_deref().is_some_and(|e| e.contains("Nora range cette boîte en ce moment : réessayez dans une minute.")),
        "{:?}",
        flow.error
    );
    // 403 on a change: the server's sentence too.
    act(&mut app, Intent::Undo("NFR-1".into()));
    reply(
        &calls,
        UNDO,
        403,
        thrown("PermissionError", "Seule la personne qui a lancé la secrétaire ou le propriétaire de la boîte peut faire cela."),
    );
    take(&mut app);
    assert!(app.nora.sorting.flow.as_ref().unwrap().error.as_deref().is_some_and(|e| e.contains("Seule la personne")));
}

#[test]
fn carrying_on_validates_the_filing_then_sets_the_rhythm_and_an_answer_goes_by_button() {
    let (mut app, calls) = with_state(false, "validated", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: true });
    act(&mut app, Intent::Continue(Some(60)));
    assert_eq!(made(&calls, VALIDATE), vec![json!({"account": BOX, "file_mails": 1})]);
    reply(&calls, VALIDATE, 200, ok(plan("Validated")));
    take(&mut app);
    assert_eq!(made(&calls, SWITCH), vec![json!({"account": BOX, "interval": 60})]);
    reply(&calls, SWITCH, 200, ok(state(true, "validated", true)));
    take(&mut app);
    assert!(app.nora.sorting.flow.as_ref().unwrap().said.as_deref().is_some_and(|s| s.contains("Every hour")));
    // « Non, quand je le demande »: nothing asked of the server.
    act(&mut app, Intent::Continue(None));
    assert_eq!(made(&calls, VALIDATE).len(), 1);
}

/// Two questions of two mailboxes, as `nora.api.asks.mine` says them.
fn mine() -> Value {
    json!({"asks": [
        {"name": "ASK-0001", "kind": "Family", "topic": "Task", "title": "Payment notices from your bank: shall I make a task of each?",
         "context": "3 this month", "choices": [{"code": "task", "label": "Yes, a task"}, {"code": "none", "label": "No, file them"}], "box": BOX},
        {"name": "ASK-0002", "kind": "Sender", "topic": "Side", "title": "Is Garage Exemple a client or a supplier?",
         "context": "", "choices": [{"code": "client", "label": "A client"}, {"code": "supplier", "label": "A supplier"}], "box": "team@example.org"}
    ], "retained": 2})
}

/// The window with Nora's questions said.
fn with_questions() -> (MailApp, Calls) {
    let (mut app, calls) = hosted();
    take(&mut app);
    assert_eq!(made(&calls, MINE), vec![json!({})], "all the person's questions, of all their mailboxes");
    reply(&calls, MINE, 200, ok(mine()));
    take(&mut app);
    assert_eq!(app.nora.sorting.questions.list.len(), 2);
    (app, calls)
}

#[test]
fn nora_s_questions_are_read_when_the_window_starts_every_fifteen_minutes_and_when_her_pane_opens() {
    let (mut app, calls) = with_questions();
    take(&mut app);
    assert_eq!(made(&calls, MINE).len(), 1, "not again at once: each read shows new ones");
    // Her pane opens a minute later: read again; a moment later: not.
    app.nora.sorting.questions.asked_at -= QUESTIONS_FRESH_MS + 1.0;
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    take(&mut app);
    assert_eq!(made(&calls, MINE).len(), 2);
    reply(&calls, MINE, 200, ok(json!({"asks": [], "retained": 2})));
    take(&mut app);
    assert!(app.nora.sorting.questions.list.is_empty());
    app.run("ui.noraAssistant", json!({"open": false})).unwrap();
    app.run("ui.noraAssistant", json!({"open": true})).unwrap();
    take(&mut app);
    assert_eq!(made(&calls, MINE).len(), 2);
    // Fifteen minutes on.
    app.nora.sorting.questions.asked_at -= QUESTIONS_EVERY_MS;
    take(&mut app);
    assert_eq!(made(&calls, MINE).len(), 3);
    // A person who has none to answer (a portal account): none shown.
    reply(&calls, MINE, 403, thrown("PermissionError", "Not permitted"));
    take(&mut app);
    assert!(app.nora.sorting.questions.list.is_empty());
}

#[test]
fn a_question_answered_by_a_button_goes_and_nora_says_what_she_noted() {
    let (mut app, calls) = with_questions();
    act(&mut app, Intent::Answer("ASK-0001".into(), "task".into()));
    assert_eq!(made(&calls, ANSWER), vec![json!({"name": "ASK-0001", "choice": "task", "channel": "button"})]);
    reply(&calls, ANSWER, 200, ok(json!({"done": true, "said": "Noted: Yes, a task", "corrected": "2 notices made into tasks.", "run": ""})));
    take(&mut app);
    let questions = &app.nora.sorting.questions;
    assert_eq!(questions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["ASK-0002"]);
    let noted = questions.noted.clone().unwrap();
    assert_eq!((noted.words.as_str(), noted.run.as_str()), ("Noted: Yes, a task 2 notices made into tasks.", ""), "nothing moved: nothing to undo");
}

#[test]
fn an_answer_in_ones_own_words_is_confirmed_as_nora_would_keep_it() {
    let (mut app, calls) = with_questions();
    // Nothing typed: nothing sent.
    act(&mut app, Intent::SayAnswer("ASK-0002".into()));
    assert!(made(&calls, ANSWER).is_empty());
    app.nora.sorting.questions.drafts.insert("ASK-0002".into(), "  it repairs  our vans, a supplier ".into());
    act(&mut app, Intent::SayAnswer("ASK-0002".into()));
    assert_eq!(made(&calls, ANSWER), vec![json!({"name": "ASK-0002", "text": "it repairs our vans, a supplier", "channel": "text"})]);
    reply(&calls, ANSWER, 200, ok(json!({"confirm": {"retained": "Garage Exemple is a supplier.", "choice": "supplier", "scope": "sender"}})));
    take(&mut app);
    let confirming = app.nora.sorting.questions.confirming.get("ASK-0002").cloned().unwrap();
    assert_eq!(confirming, Confirming { retained: "Garage Exemple is a supplier.".into(), choice: "supplier".into(), scope: "sender".into() });
    // « Dire autrement » puts it aside; the person changes Nora's sentence and confirms it.
    act(&mut app, Intent::Rephrase("ASK-0002".into()));
    assert!(app.nora.sorting.questions.confirming.is_empty());
    app.nora
        .sorting
        .questions
        .confirming
        .insert("ASK-0002".into(), Confirming { retained: "Garage Exemple repairs our vans: a supplier.".into(), ..confirming });
    act(&mut app, Intent::ConfirmAnswer("ASK-0002".into()));
    assert_eq!(
        made(&calls, CONFIRM),
        vec![json!({"name": "ASK-0002", "retained": "Garage Exemple repairs our vans: a supplier.", "choice": "supplier", "scope": "sender"})]
    );
    reply(&calls, CONFIRM, 200, ok(json!({"done": true, "said": "Noted: Garage Exemple repairs our vans: a supplier.", "corrected": ""})));
    take(&mut app);
    let questions = &app.nora.sorting.questions;
    assert!(questions.list.iter().all(|a| a.name != "ASK-0002") && questions.drafts.is_empty() && questions.confirming.is_empty());
    assert_eq!(questions.noted.as_ref().map(|n| n.words.as_str()), Some("Noted: Garage Exemple repairs our vans: a supplier."));
}

#[test]
fn later_puts_a_question_off_and_a_question_that_isnt_the_persons_goes_without_a_word() {
    let (mut app, calls) = with_questions();
    act(&mut app, Intent::Later("ASK-0001".into()));
    assert_eq!(made(&calls, LATER), vec![json!({"name": "ASK-0001"})]);
    reply(&calls, LATER, 200, ok(json!({"ok": true})));
    take(&mut app);
    assert_eq!(app.nora.sorting.questions.list.len(), 1);
    // 403: not the person's to answer (the person in charge changed meanwhile): it goes, nothing said.
    act(&mut app, Intent::Answer("ASK-0002".into(), "client".into()));
    reply(&calls, ANSWER, 403, thrown("PermissionError", "Only the person in charge of this mailbox answers Nora's questions."));
    take(&mut app);
    assert!(app.nora.sorting.questions.list.is_empty() && app.nora.sorting.questions.error.is_none());
    // 417: the server's words.
    let (mut app, calls) = with_questions();
    act(&mut app, Intent::Answer("ASK-0001".into(), "maybe".into()));
    reply(&calls, ANSWER, 417, thrown("ValidationError", "Cette réponse n'est pas une de celles que Nora proposait."));
    take(&mut app);
    assert_eq!(app.nora.sorting.questions.list.len(), 2);
    assert!(app.nora.sorting.questions.error.as_deref().is_some_and(|e| e.contains("Cette réponse n'est pas une de celles que Nora proposait.")));
}

#[test]
fn the_banner_reads_the_last_run_once_and_says_it() {
    let (mut app, calls) = hosted();
    take(&mut app);
    let mut on = state(true, "validated", true);
    on["last_run"] = json!({"run": "NFR-3", "kind": "reading", "status": "done", "started_at": "2026-10-14 15:30:00", "filed": 29, "seconds": 2.0});
    reply(&calls, STATE, 200, ok(on));
    take(&mut app);
    take(&mut app);
    assert_eq!(made(&calls, JOURNAL), vec![json!({"account": BOX, "limit": 1})]);
    reply(&calls, JOURNAL, 200, ok(journal("NFR-3")));
    take(&mut app);
    take(&mut app);
    assert_eq!(made(&calls, JOURNAL).len(), 1, "read once for that run");
    let latest = app.nora.sorting.boxes.get(BOX).and_then(|k| k.latest.clone()).unwrap();
    let today = app.session.today();
    let said = super::super::sorting_view::run_summary(&latest, today);
    assert_eq!(said, "Today at 15:55: 7 notifications, 2 newsletters, 5 messages from clients and 9 invoices filed.");
    let yesterday = Run { started_at: "2026-10-13 09:05:00".into(), ..latest.clone() };
    assert!(super::super::sorting_view::run_summary(&yesterday, today).starts_with("On 10/13 at 09:05: "));
}

#[test]
fn every_call_is_one_of_the_methods_with_a_plain_object() {
    let (mut app, calls) = with_state(false, "", true);
    act(&mut app, Intent::Open { account: BOX.into(), journal: false });
    act(&mut app, Intent::Switch { account: BOX.into(), on: true });
    act(&mut app, Intent::Activate);
    for (call, _) in calls.0.borrow().iter() {
        assert!(SORTING_METHODS.contains(&call.method), "{}", call.method);
        let args = call.args.as_object().unwrap();
        assert!(!args.contains_key("cmd"));
        assert!(call.args.to_string().len() < 8 * 1024);
    }
    assert_eq!(every(5), "Every 5 minutes");
    assert_eq!(every(15), "Every 15 minutes");
    assert_eq!(every(60), "Every hour");
    assert_eq!(every(120), "Every 2 hours");
}

#[test]
fn an_answer_to_a_question_of_a_familys_folder_moves_what_was_filed_and_annuler_undoes_it() {
    let (mut app, calls) = hosted();
    take(&mut app);
    reply(
        &calls,
        MINE,
        200,
        ok(json!({"asks": [{"name": "ASK-0007", "kind": "Family", "topic": "Folder",
            "title": "« Payment notice no. 117 »: shall I put them in « Invoices received » or in « Notifications »?",
            "context": "In the last 30 days I filed 4 in « Invoices received » and 3 in « Notifications ».",
            "choices": [{"code": "folder_1", "label": "Invoices received"}, {"code": "folder_2", "label": "Notifications"}, {"code": "inbox", "label": "Leave them in the inbox"}],
            "box": BOX}], "retained": 0})),
    );
    take(&mut app);
    act(&mut app, Intent::Answer("ASK-0007".into(), "folder_2".into()));
    assert_eq!(made(&calls, ANSWER), vec![json!({"name": "ASK-0007", "choice": "folder_2", "channel": "button"})]);
    reply(
        &calls,
        ANSWER,
        200,
        ok(
            json!({"done": true, "said": "Noted: Notifications", "corrected": "4 mails of this family were moved to « Notifications ».", "run": "NFR-C1"}),
        ),
    );
    take(&mut app);
    let noted = app.nora.sorting.questions.noted.clone().unwrap();
    assert_eq!((noted.account.as_str(), noted.run.as_str()), (BOX, "NFR-C1"));
    assert!(app.nora.sorting.questions.list.is_empty());
    // « Annuler »: the correction undone in its mailbox, called again while some are left.
    act(&mut app, Intent::UndoNoted);
    assert_eq!(made(&calls, UNDO), vec![json!({"account": BOX, "run": "NFR-C1"})]);
    reply(&calls, UNDO, 200, ok(json!({"undone": 3, "moved": 0, "gone": 0, "failed": 0, "remaining": 1})));
    take(&mut app);
    assert_eq!(made(&calls, UNDO).len(), 2);
    reply(&calls, UNDO, 200, ok(json!({"undone": 1, "moved": 0, "gone": 0, "failed": 0, "remaining": 0})));
    take(&mut app);
    let noted = app.nora.sorting.questions.noted.clone().unwrap();
    assert_eq!((noted.words.as_str(), noted.run.as_str()), ("1 message is back where it was.", ""));
    // The journal's correction: what it was about.
    let journal = read_journal(&json!({"runs": [{"run": "NFR-C1", "kind": "correction", "ask": "ASK-0007", "about": "Payment notice no. 117",
        "status": "undone", "started_at": "2026-10-14 15:58:00", "filed": 4, "still_filed": 0, "folders": [], "kinds": {"notifications": 4, "inbox": 0}, "can_undo": false}]}));
    assert_eq!((journal.runs[0].kind.as_str(), journal.runs[0].about.as_str()), ("correction", "Payment notice no. 117"));
}
