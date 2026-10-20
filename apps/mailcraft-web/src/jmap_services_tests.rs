// //// Neoffice — added file (no upstream equivalent): the tests of the browser's JMAP runner.

//! The runner of a JMAP account's Send/Receive and downloads, over the in-process JMAP server: the job's
//! own credential or the host's, a credential the server refuses (at sign-in or during the run) and the
//! host renews, a host that has none yet, and a run that is stopped. Nothing here leaves this machine.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_jmap::standin::{Calendar, Event, Pim, SESSION, StandIn, TOKEN};
use mailcraft_jmap::{Auth, Http, Request, Response, block_on};
use mailcraft_model::remote::CalendarAsk;
use mailcraft_model::sync::{JmapDraftUp, JmapFetchRequest, JmapMailRequest, JmapMailResult, JmapPimRequest, JmapPimResult, JmapScope, JmapSend};
use mailcraft_model::{Address, Body, Message};
use mailcraft_ui_egui::jmap_job::{JmapFetchJob, JmapJob};
use mailcraft_ui_egui::jmap_pim::JmapPimJob;

use super::{ASK_AGAIN_MS, AT_ONCE, FACADE_API, Queue, fetch, pim, run};
use crate::credentials::{Credentials, Renewal};

const ME: &str = "ada@example.org";

/// A host that holds a credential and answers each renewal from a list (`Reload` once it is empty), and, when it
/// gives a new token, makes the server take it, as the page and its server do together.
struct Host {
    held: RefCell<Option<Auth>>,
    answers: RefCell<VecDeque<Renewal>>,
    asked: Cell<u32>,
    server: StandIn,
}

impl Host {
    fn new(server: &StandIn, held: Option<&str>, answers: Vec<Renewal>) -> Host {
        Host { held: RefCell::new(held.map(Auth::bearer)), answers: RefCell::new(answers.into()), asked: Cell::new(0), server: server.clone() }
    }
}

impl Credentials for Host {
    fn auth(&self, _email: &str) -> Option<Auth> {
        self.held.borrow().clone()
    }

    async fn renew(&self, _email: &str) -> Renewal {
        self.asked.set(self.asked.get() + 1);
        let answer = self.answers.borrow_mut().pop_front().unwrap_or(Renewal::Reload);
        if let Renewal::Given(Auth::Bearer(token)) = &answer {
            self.server.with(|d| d.token = Some(token.clone()));
            *self.held.borrow_mut() = Some(Auth::bearer(token.clone()));
        }
        answer
    }
}

/// A server with an Inbox of two messages.
fn server() -> StandIn {
    let server = StandIn::new();
    server.with(|d| {
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        for (n, received) in [(1, "2026-10-15T09:30:00Z"), (2, "2026-10-16T09:30:00Z")] {
            let raw =
                format!("From: Zoe <zoe@example.org>\r\nTo: Ada <{ME}>\r\nSubject: Hello {n}\r\nMessage-ID: <m{n}@example.org>\r\n\r\nHello.\r\n");
            d.deliver(&inbox, raw.as_bytes(), &[], received);
        }
    });
    server
}

/// A job for a run that lists the mailboxes and reads the Inbox, with the job's own secret or none.
fn job(secret: Option<&str>) -> JmapJob {
    let request = JmapMailRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        window: 50,
        list_folders: true,
        scope: JmapScope::Light { also: Vec::new() },
        ..Default::default()
    };
    JmapJob { request, secret: secret.map(str::to_string), cancel: Arc::new(AtomicBool::new(false)) }
}

/// Every piece of a run, and the waits it asked for, in milliseconds.
fn pieces(server: &StandIn, job: &JmapJob, host: Option<&Host>) -> (Vec<JmapMailResult>, Vec<i32>) {
    let waits = RefCell::new(Vec::new());
    let mut all = Vec::new();
    let sleep = |ms: i32| {
        waits.borrow_mut().push(ms);
        std::future::ready(())
    };
    block_on(run(server.clone(), job, host, sleep, &mut |piece| all.push(piece)));
    (all, waits.into_inner())
}

/// The last piece: the only one that isn't `partial`, and the last of all.
fn last(pieces: &[JmapMailResult]) -> &JmapMailResult {
    let (last, rest) = pieces.split_last().expect("a run ends with a piece");
    assert!(!last.partial && rest.iter().all(|piece| piece.partial), "{pieces:?}");
    assert!(pieces.iter().all(|piece| (piece.account, piece.email.as_str()) == (7, ME)), "{pieces:?}");
    last
}

/// The messages the windows of a run brought.
fn messages(pieces: &[JmapMailResult]) -> usize {
    pieces.iter().flat_map(|piece| &piece.pages).map(|page| page.messages.len()).sum()
}

#[test]
fn a_run_with_the_host_s_credential_reads_the_inbox() {
    let server = server();
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let (all, waits) = pieces(&server, &job(None), Some(&host));
    let end = last(&all);
    assert!(end.whole && !end.auth_refused && !end.stopped, "{end:?}");
    assert!(all.first().is_some_and(|piece| piece.listing.is_some()), "the mailboxes first");
    assert_eq!(messages(&all), 2);
    assert_eq!((host.asked.get(), waits.len()), (0, 0), "nothing to renew");
}

#[test]
fn a_token_refused_at_sign_in_is_renewed_by_the_host_and_the_run_goes_on() {
    let server = server();
    // The host's token has expired: the server holds another one now.
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let (all, _) = pieces(&server, &job(None), Some(&host));
    let end = last(&all);
    assert!(end.whole && !end.auth_refused, "{end:?}");
    assert_eq!(messages(&all), 2);
    assert_eq!(host.asked.get(), 1);
}

#[test]
fn a_token_that_expires_during_the_run_is_renewed_and_the_run_is_read_again() {
    let server = server();
    // The session and the list of mailboxes go through; the next request finds the token gone.
    server.with(|d| d.drop_token_after = Some(1));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let (all, _) = pieces(&server, &job(None), Some(&host));
    let end = last(&all);
    assert!(end.whole && !end.auth_refused, "{end:?}");
    assert_eq!(all.iter().filter(|piece| !piece.partial).count(), 1, "one last piece, the refusal held back");
    assert_eq!(messages(&all), 2);
    assert_eq!(host.asked.get(), 1);
}

#[test]
fn a_run_read_again_after_a_renewal_sends_nothing_a_second_time() {
    // A server with a Sent and a Drafts, who the account is, and a token that dies after the first message went.
    let server = StandIn::new();
    server.with(|d| {
        d.mailbox("Inbox", None, Some("inbox"));
        d.mailbox("Sent", None, Some("sent"));
        d.mailbox("Drafts", None, Some("drafts"));
        d.identity("Ada", ME);
        // The list of mailboxes, who the account is, the first message's filing and its submission go through;
        // the next request finds the token gone.
        d.drop_token_after = Some(4);
    });
    let letter = |id: u64, subject: &str| Message {
        id,
        from: Address::new("Ada", ME),
        to: vec![Address::new("Bob", "bob@example.org")],
        subject: subject.into(),
        message_id: format!("{subject}@example.org"),
        date: 1_791_993_600,
        body: Body { text: format!("About {subject}."), html: None },
        ..Default::default()
    };
    let request = |sends: &[(u64, &str)], drafts: &[u64]| {
        let mut asks = job(None);
        asks.request.sends =
            sends.iter().map(|&(id, subject)| JmapSend { message: id, draft_copy: String::new(), mime: Box::new(letter(id, subject)) }).collect();
        asks.request.drafts =
            drafts.iter().map(|&id| JmapDraftUp { message: id, saves: 1, replaces: String::new(), mime: Box::new(letter(id, "draft")) }).collect();
        asks
    };
    let sent_of = |pieces: &[JmapMailResult]| pieces.iter().flat_map(|piece| piece.sent.iter().map(|done| done.message)).collect::<Vec<_>>();
    let filed_of = |pieces: &[JmapMailResult]| pieces.iter().flat_map(|piece| piece.drafts.iter().map(|done| done.message)).collect::<Vec<_>>();
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let (all, _) = pieces(&server, &request(&[(7, "first"), (8, "second")], &[9]), Some(&host));
    assert!(last(&all).whole && !last(&all).auth_refused, "{:?}", last(&all));
    assert_eq!(host.asked.get(), 1);
    // The first went once; the run that was read again only read, so the second and the draft wait.
    assert_eq!(sent_of(&all), [7]);
    assert_eq!(filed_of(&all), Vec::<u64>::new());
    assert_eq!(server.with(|d| d.delivered.len()), 1);
    assert_eq!(server.with(|d| d.emails.iter().filter(|e| e.keywords.contains("$draft")).count()), 0);
    // The next run, with what the engine still has waiting, sends the second and files the draft.
    let (next, _) = pieces(&server, &request(&[(8, "second")], &[9]), Some(&host));
    assert!(last(&next).whole, "{:?}", last(&next));
    assert_eq!((sent_of(&next), filed_of(&next)), (vec![8], vec![9]));
    let sent = server.with(|d| d.delivered.iter().map(|d| String::from_utf8_lossy(&d.raw).to_string()).collect::<Vec<_>>());
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].contains("Subject: first") && sent[1].contains("Subject: second"));
    assert_eq!(server.with(|d| d.emails.iter().filter(|e| e.keywords.contains("$draft")).count()), 1);
    assert_eq!(server.with(|d| d.emails.iter().filter(|e| !e.keywords.contains("$draft")).count()), 2, "two copies in Sent");
    assert_eq!(host.asked.get(), 1, "the fresh token served the next run");
}

#[test]
fn a_host_with_nothing_yet_is_asked_again_a_little_later() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Later, Renewal::Later, Renewal::Given(Auth::bearer("fresh"))]);
    let (all, waits) = pieces(&server, &job(None), Some(&host));
    assert!(last(&all).whole);
    // Waited 5 s, then 10 s, in slices that let a stop through.
    assert_eq!(waits.iter().sum::<i32>(), ASK_AGAIN_MS[..2].iter().sum::<i32>());
    assert!(waits.iter().all(|ms| *ms <= 500), "{waits:?}");
    assert_eq!(host.asked.get(), 3);
}

#[test]
fn a_host_that_never_has_one_leaves_the_refusal_standing() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    // Asked once, then once after each wait: then the run ends.
    let host = Host::new(&server, Some(TOKEN), (0..10).map(|_| Renewal::Later).collect());
    let (all, waits) = pieces(&server, &job(None), Some(&host));
    let end = last(&all);
    assert!(end.auth_refused && !end.whole, "{end:?}");
    assert_eq!(waits.iter().sum::<i32>(), ASK_AGAIN_MS.iter().sum::<i32>());
    assert_eq!(host.asked.get(), 4);
    // A host that says it has nothing more to give is asked once.
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Reload]);
    let (all, waits) = pieces(&server, &job(None), Some(&host));
    assert!(last(&all).auth_refused);
    assert_eq!((host.asked.get(), waits.len()), (1, 0));
}

/// A host whose new tokens the server never takes: the server moves on to another one each time.
struct Outrun<'a>(&'a Host);

impl Credentials for Outrun<'_> {
    fn auth(&self, email: &str) -> Option<Auth> {
        self.0.auth(email)
    }

    async fn renew(&self, email: &str) -> Renewal {
        let answer = self.0.renew(email).await;
        self.0.server.with(|d| d.token = Some("rotated-again".into()));
        answer
    }
}

#[test]
fn a_renewed_token_that_is_refused_too_is_not_renewed_again() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh")), Renewal::Given(Auth::bearer("fresher"))]);
    let mut all = Vec::new();
    block_on(run(server.clone(), &job(None), Some(&Outrun(&host)), |_| std::future::ready(()), &mut |piece| all.push(piece)));
    assert!(last(&all).auth_refused);
    assert_eq!(host.asked.get(), 1, "one renewal a run");
}

#[test]
fn the_owner_s_own_credential_refused_is_never_renewed() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some("rotated"), vec![Renewal::Given(Auth::bearer("fresh"))]);
    // The job carries the owner's own token: the host is not asked, even when it could answer.
    let (all, _) = pieces(&server, &job(Some(TOKEN)), Some(&host));
    assert!(last(&all).auth_refused);
    assert_eq!(host.asked.get(), 0);
    // And with the right one it goes through.
    let (all, _) = pieces(&server, &job(Some("rotated")), Some(&host));
    assert!(last(&all).whole);
}

#[test]
fn without_any_credential_the_run_ends_refused() {
    let server = server();
    let host = Host::new(&server, None, Vec::new());
    let (all, _) = pieces(&server, &job(None), Some(&host));
    assert!(last(&all).auth_refused);
    let (all, _) = pieces(&server, &job(None), None);
    assert!(last(&all).auth_refused);
    assert!(server.seen().is_empty(), "nothing was asked of the server, not even its session");
}

#[test]
fn a_run_stopped_before_it_starts_or_while_it_waits_says_so() {
    let server = server();
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let stopped = job(None);
    stopped.cancel.store(true, Ordering::Relaxed);
    let (all, _) = pieces(&server, &stopped, Some(&host));
    let end = last(&all);
    assert!(end.stopped && !end.auth_refused && !end.whole, "{end:?}");
    assert!(server.seen().is_empty(), "not even the session");
    // Stopped while it waits for the host: stopped, not refused.
    server.with(|d| d.token = Some("rotated".into()));
    let waiting = job(None);
    let cancel = waiting.cancel.clone();
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Later, Renewal::Later]);
    let mut all = Vec::new();
    let sleep = |_ms: i32| {
        cancel.store(true, Ordering::Relaxed);
        std::future::ready(())
    };
    block_on(run(server.clone(), &waiting, Some(&host), sleep, &mut |piece| all.push(piece)));
    let end = last(&all);
    assert!(end.stopped && !end.auth_refused, "{end:?}");
    assert_eq!(host.asked.get(), 1);
}

#[test]
fn a_password_goes_with_the_account_s_address() {
    assert_eq!(super::own_auth(false, ME, "pw"), Auth::basic(ME, "pw"));
    assert_eq!(super::own_auth(true, ME, "tok"), Auth::bearer("tok"));
}

/// A job that downloads the server's emails whole, with the job's own secret or none.
fn fetch_job(server: &StandIn, secret: Option<&str>) -> JmapFetchJob {
    let ids = server.with(|d| d.emails.iter().map(|email| email.id.clone()).collect());
    let request = JmapFetchRequest { account: 7, email: ME.into(), session: SESSION.into(), bearer: true, ids, background: false };
    JmapFetchJob { request, secret: secret.map(str::to_string), cancel: Arc::new(AtomicBool::new(false)) }
}

#[test]
fn a_download_with_the_host_s_credential_brings_the_messages_whole() {
    let server = server();
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let answer = block_on(fetch(server.clone(), &fetch_job(&server, None), Some(&host), |_| std::future::ready(())));
    assert_eq!(answer.fetched.len(), 2, "{answer:?}");
    assert!(answer.fetched.iter().all(|message| String::from_utf8_lossy(&message.mime).contains("Subject: Hello")));
    assert!(answer.errors.is_empty() && answer.missing.is_empty() && (answer.account, answer.email.as_str()) == (7, ME));
    assert_eq!(host.asked.get(), 0);
}

#[test]
fn a_download_whose_token_is_refused_signs_in_again_once_with_a_new_one() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let answer = block_on(fetch(server.clone(), &fetch_job(&server, None), Some(&host), |_| std::future::ready(())));
    assert_eq!((answer.fetched.len(), host.asked.get()), (2, 1), "{answer:?}");
    // A host with nothing to give: nothing fetched, and why
    server.with(|d| d.token = Some("rotated-again".into()));
    let host = Host::new(&server, Some("fresh"), vec![Renewal::Reload]);
    let answer = block_on(fetch(server.clone(), &fetch_job(&server, None), Some(&host), |_| std::future::ready(())));
    assert!(answer.fetched.is_empty() && !answer.errors.is_empty(), "{answer:?}");
    // The owner's own token refused: the host is never asked
    let host = Host::new(&server, Some("rotated-again"), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let answer = block_on(fetch(server.clone(), &fetch_job(&server, Some(TOKEN)), Some(&host), |_| std::future::ready(())));
    assert!(answer.fetched.is_empty() && !answer.errors.is_empty(), "{answer:?}");
    assert_eq!(host.asked.get(), 0);
}

#[test]
fn a_run_refused_mid_way_and_stopped_while_it_waits_ends_stopped() {
    let server = server();
    // The session and the list of mailboxes go through; then the token is gone, and the host has none yet.
    server.with(|d| d.drop_token_after = Some(1));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Later, Renewal::Later]);
    let waiting = job(None);
    let cancel = waiting.cancel.clone();
    let mut all = Vec::new();
    // The owner stops the run while it waits for the host.
    let sleep = |_ms: i32| {
        cancel.store(true, Ordering::Relaxed);
        std::future::ready(())
    };
    block_on(run(server.clone(), &waiting, Some(&host), sleep, &mut |piece| all.push(piece)));
    let end = last(&all);
    assert!(end.stopped && !end.auth_refused && !end.whole, "{end:?}");
    assert_eq!(all.iter().filter(|piece| !piece.partial).count(), 1, "one last piece");
}

#[test]
fn a_download_whose_renewed_token_is_refused_too_asks_once() {
    let server = server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh")), Renewal::Given(Auth::bearer("fresher"))]);
    let answer = block_on(fetch(server.clone(), &fetch_job(&server, None), Some(&Outrun(&host)), |_| std::future::ready(())));
    assert!(answer.fetched.is_empty() && !answer.errors.is_empty(), "{answer:?}");
    assert_eq!(host.asked.get(), 1, "one renewal a download");
    // Stopped while it waits for the host: nothing fetched, and nothing said against the server.
    server.with(|d| d.token = Some("rotated-again".into()));
    let host = Host::new(&server, Some("fresh"), vec![Renewal::Later]);
    let stopped = fetch_job(&server, None);
    let cancel = stopped.cancel.clone();
    let sleep = |_ms: i32| {
        cancel.store(true, Ordering::Relaxed);
        std::future::ready(())
    };
    let answer = block_on(fetch(server.clone(), &stopped, Some(&host), sleep));
    assert!(answer.fetched.is_empty() && answer.errors.is_empty() && !answer.offline, "{answer:?}");
}

#[test]
fn the_gate_lets_requests_go_in_the_order_they_came() {
    let mut queue = Queue::default();
    let (first, second, third) = (queue.take(), queue.take(), queue.take());
    // Not out of turn, and never more than its room.
    assert!(!queue.go(second), "the second waits for the first");
    assert!(queue.go(first) && queue.go(second));
    assert_eq!(AT_ONCE, 2);
    assert!(!queue.go(third), "no room for a third");
    // A request that was on its way takes a new ticket for its next one: behind the third.
    queue.leave(first, true);
    let again = queue.take();
    assert!(!queue.go(again), "the one that waited goes first");
    assert!(queue.go(third));
    // A request that gives up waiting leaves the queue, and the one after it moves up.
    let gives_up = queue.take();
    let after = queue.take();
    queue.leave(gives_up, false);
    queue.leave(second, true);
    assert!(!queue.go(after), "the one before it first");
    assert!(queue.go(again));
    queue.leave(third, true);
    assert!(queue.go(after));
}

/// A server with an Inbox and Ada's calendar, which holds a meeting on Thursday.
fn pim_server() -> StandIn {
    let server = server();
    server.with(|d| {
        d.pim = Pim::offered();
        d.pim.add_calendar(Calendar { flagged: true, ..Calendar::new("ada", "Ada", true) });
        d.pim.add_event(Event::timed("budget", "ada", "Budget review", "2026-10-15T09:00:00", Some("Europe/Zurich"), "PT1H"));
    });
    server
}

/// A calendar job of the days around Wednesday 2026-10-14, listing the calendars, with the job's own secret or none.
fn pim_job(secret: Option<&str>) -> JmapPimJob {
    let now = 1_791_993_600;
    let request = JmapPimRequest {
        account: 7,
        email: ME.into(),
        session: SESSION.into(),
        bearer: true,
        me: vec![ME.into()],
        now,
        from: now - 7 * 86_400,
        to: now + 14 * 86_400,
        lists: true,
        asks: vec![CalendarAsk { calendar: String::new(), series: Vec::new() }],
        ..Default::default()
    };
    JmapPimJob { request, secret: secret.map(str::to_string), cancel: Arc::new(AtomicBool::new(false)) }
}

fn pim_of(http: impl Http + Clone, job: &JmapPimJob, host: Option<&Host>) -> JmapPimResult {
    block_on(pim(http, job, host, |_| std::future::ready(())))
}

/// What a calendar job read whole: the titles of the main calendar's events.
fn read_titles(result: &JmapPimResult) -> Vec<String> {
    result.events.iter().flat_map(|d| d.events.iter().map(|e| e.title.clone())).collect()
}

#[test]
fn a_pim_job_signs_in_with_the_jobs_secret_or_the_hosts_credential() {
    let server = pim_server();
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let with_host = pim_of(server.clone(), &pim_job(None), Some(&host));
    assert_eq!(read_titles(&with_host), ["Budget review"]);
    assert!(with_host.errors.is_empty() && (with_host.account, with_host.email.as_str()) == (7, ME), "{with_host:?}");
    let own = pim_of(server.clone(), &pim_job(Some(TOKEN)), None);
    assert_eq!(read_titles(&own), ["Budget review"]);
    // The owner's own token refused: refused, and the host never asked.
    let refused = pim_of(server.clone(), &pim_job(Some("wrong")), Some(&host));
    assert!(refused.auth_refused && refused.events.is_none());
    assert_eq!(host.asked.get(), 0);
    // No credential at all: refused, and nothing asked of the server.
    let fresh = pim_server();
    assert!(pim_of(fresh.clone(), &pim_job(None), None).auth_refused);
    assert!(fresh.seen().is_empty());
}

#[test]
fn a_pim_job_whose_token_is_refused_is_renewed_once_and_read_again() {
    // At sign-in: the host's token has expired.
    let server = pim_server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let result = pim_of(server.clone(), &pim_job(None), Some(&host));
    assert_eq!((read_titles(&result), host.asked.get()), (vec!["Budget review".to_string()], 1));
    // In the middle of the job: a server that takes one call at a time, whose token dies after the list.
    let server = pim_server();
    server.with(|d| {
        d.max_calls = 1;
        d.drop_token_after = Some(1);
    });
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh"))]);
    let result = pim_of(server.clone(), &pim_job(None), Some(&host));
    assert!(!result.auth_refused && result.errors.is_empty(), "{result:?}");
    assert_eq!((read_titles(&result), host.asked.get()), (vec!["Budget review".to_string()], 1), "read again, once");
    assert!(result.events.as_ref().unwrap().complete);
}

#[test]
fn a_pim_job_whose_renewed_token_is_refused_too_answers_auth_refused() {
    let server = pim_server();
    server.with(|d| d.token = Some("rotated".into()));
    let host = Host::new(&server, Some(TOKEN), vec![Renewal::Given(Auth::bearer("fresh")), Renewal::Given(Auth::bearer("fresher"))]);
    let result = block_on(pim(server.clone(), &pim_job(None), Some(&Outrun(&host)), |_| std::future::ready(())));
    assert!(result.auth_refused, "{result:?}");
    assert_eq!(host.asked.get(), 1, "one renewal a job");
}

#[test]
fn a_pim_job_stopped_before_it_starts_says_so() {
    let server = pim_server();
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let job = pim_job(None);
    job.cancel.store(true, Ordering::Relaxed);
    let result = pim_of(server.clone(), &job, Some(&host));
    assert!(result.stopped && !result.auth_refused && result.events.is_none(), "{result:?}");
    assert!(server.seen().is_empty(), "not even the session");
}

/// The stand-in as the facade serves it: its API at the facade's method path.
#[derive(Clone)]
struct Facade(StandIn);

impl Http for Facade {
    async fn send(&self, mut req: Request) -> mailcraft_jmap::Result<Response> {
        req.url = req.url.replace(FACADE_API, "/jmap/api");
        let session = req.url.ends_with("/jmap/session");
        let mut resp = self.0.send(req).await?;
        if session {
            let text = String::from_utf8(resp.body).unwrap_or_default();
            resp.body = text.replace("\"/jmap/api\"", &format!("\"{FACADE_API}\"")).into_bytes();
        }
        Ok(resp)
    }
}

#[test]
fn a_pim_job_against_the_facades_api_takes_its_calendars_for_plain() {
    let server = pim_server();
    server.with(|d| d.pim.says_writable = false);
    let host = Host::new(&server, Some(TOKEN), Vec::new());
    let plain = |result: &JmapPimResult| result.calendars.as_ref().unwrap().calendars.iter().all(|c| c.plain);
    // Its session says nothing of what it writes: elsewhere a server is taken to keep everything, there the basics.
    assert!(!plain(&pim_of(server.clone(), &pim_job(None), Some(&host))));
    let facade = pim_of(Facade(server.clone()), &pim_job(None), Some(&host));
    assert!(plain(&facade), "{facade:?}");
    assert_eq!(read_titles(&facade), ["Budget review"]);
}
