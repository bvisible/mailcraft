// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account as the engine works out what a run reads and takes in what comes back: the
//! folder list, complete and partial windows, the keywords, and messages whose id changed.

use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, MessagePage, ServerMessage};
// //// Neoffice — upstream's test of messages fetched from the server an account named before: the fork's fetch intake.
use mailcraft_model::sync::{GraphFetchResult, GraphFetched};

use super::super::jmap_fetch::apply_jmap_fetch;
use mailcraft_model::{Address, FlagStatus, Message};
use serde_json::json;

use super::*;

const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

/// A session with one JMAP account; its id.
fn session() -> (Session, Id) {
    let mut s = Session::empty();
    // A host that runs a JMAP account's Send/Receive (this fork's browser build).
    s.jmap_runs = true;
    let added = s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap();
    (s, added["id"].as_u64().unwrap())
}

fn folder(id: &str, parent: &str, name: &str, kind: FolderKind) -> GraphFolder {
    GraphFolder { id: id.into(), parent: parent.into(), name: name.into(), kind, ..Default::default() }
}

/// The server's mailboxes: the ones with a role, and two of the owner's.
fn server() -> Vec<GraphFolder> {
    vec![
        folder("Min", "", "INBOX", FolderKind::Inbox),
        folder("Mdr", "", "Drafts", FolderKind::Drafts),
        folder("Mse", "", "Sent", FolderKind::Sent),
        folder("Mtr", "", "Trash", FolderKind::Deleted),
        folder("Mju", "", "Junk", FolderKind::Junk),
        folder("Mpr", "", "Projects", FolderKind::Custom),
        folder("Map", "Mpr", "Apollo", FolderKind::Custom),
    ]
}

fn mail(id: &str, message_id: &str, subject: &str, received: i64) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), ..Default::default() },
        message_id: message_id.into(),
        subject: subject.into(),
        from: Address { name: "Grace".into(), email: "grace@example.org".into() },
        sent: received,
        received,
        preview: format!("{subject}…"),
        ..Default::default()
    }
}

fn window(mailbox: &str, messages: Vec<GraphMessage>, complete: bool) -> MessagePage {
    MessagePage { folder: mailbox.into(), messages, afresh: complete, ..Default::default() }
}

fn take(s: &mut Session, account: Id, listing: Option<Vec<GraphFolder>>, pages: Vec<MessagePage>, whole: bool) -> Value {
    let listing = listing.map(|folders| FolderListing { folders, complete: true });
    apply_jmap_mail(s, JmapMailResult { account, email: ME.into(), listing, pages, whole, ..Default::default() }).unwrap()
}

fn in_folder<'s>(s: &'s Session, account: Id, mailbox: &str) -> Vec<&'s Message> {
    let folder = s.mailbox.folder_by_server(account, mailbox).unwrap();
    let mut found: Vec<&Message> = s.mailbox.messages.values().map(|m| m.as_ref()).filter(|m| m.folder == folder).collect();
    found.sort_by_key(|m| std::cmp::Reverse(m.received));
    found
}

fn server_id(m: &Message) -> &str {
    m.server.as_ref().map_or("", |s| s.id.as_str())
}

#[test]
fn a_jmap_account_has_a_request_of_its_own() {
    let (s, account) = session();
    let request = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert_eq!((request.account, request.email.as_str(), request.session.as_str(), request.bearer), (account, ME, SESSION, true));
    // Nothing is tied to the server yet: the whole account, with its folder list.
    assert_eq!((request.scope.clone(), request.list_folders, request.window), (JmapScope::All, true, 500));
    assert!(request.known.is_empty() && request.changes.is_empty() && request.sends.is_empty());
    // The Graph and IMAP requests are not its own; `sync.jmapRequest` is (`sync.request` hands it
    // too once such an account counts among those with a server, when the window can run it).
    assert!(super::super::graph_mail::graph_mail_request(&s, account).is_err());
    assert!(super::super::account::sync_request(&s, account).is_err());
    let mut s = s;
    let answer = s.run("sync.jmapRequest", &json!({"account": account, "full": true})).unwrap();
    assert_eq!((answer["session"].as_str(), answer["window"].as_u64(), answer["listFolders"].as_bool()), (Some(SESSION), Some(500), Some(true)));
    // An account of another kind is refused.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap();
    assert!(jmap_mail_request(&s, imap["id"].as_u64().unwrap(), false, &[], false).is_err());
}

#[test]
fn the_window_is_the_accounts_setting_at_most_500() {
    let (mut s, account) = session();
    for (limit, window) in [(0, 500), (50, 50), (500, 500), (10_000, 500)] {
        s.prefs.sync_limit = limit;
        assert_eq!(jmap_mail_request(&s, account, false, &[], false).unwrap().window, window, "{limit}");
    }
}

#[test]
fn the_folder_list_ties_the_standard_folders_by_role_and_makes_the_others() {
    let (mut s, account) = session();
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let answer = take(&mut s, account, Some(server()), vec![], true);
    // The account's Inbox is the server's, by role, under the server's name.
    assert_eq!(s.mailbox.folder_by_server(account, "Min"), Some(inbox));
    let projects = s.mailbox.folder_by_server(account, "Mpr").unwrap();
    let apollo = s.mailbox.folder_by_server(account, "Map").unwrap();
    assert_eq!(s.mailbox.folder(apollo).unwrap().parent, Some(projects));
    assert!(answer["added"].as_u64().unwrap() >= 2, "{answer}");
    // Tied now: a run without `full` reads the Inbox and what is asked for, not the folder list.
    let request = jmap_mail_request(&s, account, false, &["Map".into(), "not an id".into()], true).unwrap();
    assert_eq!(request.scope, JmapScope::Light { also: vec!["Map".into()] });
    assert!(!request.list_folders && request.background);
    assert_eq!(jmap_mail_request(&s, account, true, &[], false).unwrap().scope, JmapScope::All);
}

#[test]
fn a_complete_window_makes_the_mailbox_the_same_as_the_servers() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    let first = vec![mail("E1", "<1@x>", "One", 100), mail("E2", "<2@x>", "Two", 200), mail("E3", "<3@x>", "Three", 300)];
    let answer = take(&mut s, account, None, vec![window("Min", first, true)], true);
    assert_eq!(answer["new"], 3);
    let inbox = in_folder(&s, account, "Min");
    assert_eq!(inbox.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Three", "Two", "One"]);
    // Headers and a preview, not the whole message.
    assert!(inbox.iter().all(|m| m.unfetched && m.body.text.ends_with('…')));
    // Known now: the next request names them, newest first, so the host asks only for their keywords.
    let known = jmap_mail_request(&s, account, false, &[], false).unwrap().known;
    assert_eq!(known, vec![JmapKnown { mailbox: "Min".into(), ids: vec!["E3".into(), "E2".into(), "E1".into()] }]);
    // The server read one and lost another: a complete window says so.
    let mut two = mail("E2", "<2@x>", "Two", 200);
    two.server.read = true;
    let answer = take(&mut s, account, None, vec![window("Min", vec![mail("E3", "<3@x>", "Three", 300), two], true)], true);
    assert_eq!((answer["changed"].as_u64(), answer["removedMessages"].as_u64()), (Some(1), Some(1)), "{answer}");
    let inbox = in_folder(&s, account, "Min");
    assert_eq!(inbox.iter().map(|m| (server_id(m), m.read)).collect::<Vec<_>>(), [("E3", false), ("E2", true)]);
}

#[test]
fn a_new_message_shows_the_marks_the_server_gave_it() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    let mut answered = mail("E1", "<1@x>", "One", 100);
    (answered.server.read, answered.server.answered, answered.server.forwarded, answered.server.flag) = (true, true, true, FlagStatus::Flagged);
    let mut plain = mail("E2", "<2@x>", "Two", 200);
    plain.draft = true;
    take(&mut s, account, None, vec![window("Min", vec![answered, plain], true)], true);
    let inbox = in_folder(&s, account, "Min");
    let marks: Vec<(&str, bool, bool, bool, FlagStatus, bool)> =
        inbox.iter().map(|m| (m.subject.as_str(), m.read, m.answered, m.forwarded, m.flag, m.draft)).collect();
    assert_eq!(marks, [("Two", false, false, false, FlagStatus::None, true), ("One", true, true, true, FlagStatus::Flagged, false)]);
    // What the message shows is what the server last said of it: nothing is waiting to go up.
    for m in inbox {
        let there = m.server.as_ref().unwrap();
        assert_eq!((m.read, m.answered, m.forwarded, m.flag), (there.read, there.answered, there.forwarded, there.flag));
    }
}

#[test]
fn a_message_that_is_its_marks_alone_and_that_the_mailbox_no_longer_has_is_not_added_blank() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    take(&mut s, account, None, vec![window("Min", vec![mail("E1", "<1@x>", "One", 100), mail("E2", "<2@x>", "Two", 200)], true)], true);
    let one = in_folder(&s, account, "Min").iter().find(|m| m.subject == "One").map(|m| m.id).unwrap();
    // The host was told the mailbox has both and sends the marks of both; One is gone from here before they are taken in,
    // and nothing waits to delete it on the server (a delete for good would: see `deletes_for_good_are_not_brought_down`).
    s.mailbox.forget_message(one).unwrap();
    let marks = |id: &str, read: bool| GraphMessage { server: ServerMessage { id: id.into(), read, ..Default::default() }, ..Default::default() };
    let answer = take(&mut s, account, None, vec![window("Min", vec![marks("E2", true), marks("E1", true)], true)], true);
    assert_eq!((answer["new"].as_u64(), answer["changed"].as_u64()), (Some(0), Some(1)), "{answer}");
    assert_eq!(in_folder(&s, account, "Min").iter().map(|m| (m.subject.as_str(), m.read)).collect::<Vec<_>>(), [("Two", true)]);
    // The next run does not name it as one the mailbox has; the server lists it, and it is added whole.
    let known = jmap_mail_request(&s, account, false, &[], false).unwrap().known;
    assert_eq!(known, vec![JmapKnown { mailbox: "Min".into(), ids: vec!["E2".into()] }]);
    let answer = take(&mut s, account, None, vec![window("Min", vec![mail("E2", "<2@x>", "Two", 200), mail("E1", "<1@x>", "One", 100)], true)], true);
    assert_eq!(answer["new"], 1, "{answer}");
    assert_eq!(in_folder(&s, account, "Min").iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Two", "One"]);
    // A message the server has whole is never taken for marks alone, however little else it says of itself: it has a size.
    let bare = GraphMessage { server: ServerMessage { id: "E7".into(), size: 120, ..Default::default() }, ..Default::default() };
    let answer = take(&mut s, account, None, vec![window("Min", vec![bare], false)], true);
    assert_eq!(answer["new"], 1, "{answer}");
    assert!(in_folder(&s, account, "Min").iter().any(|m| server_id(m) == "E7"));
}

#[test]
fn a_window_cut_short_adds_and_updates_but_removes_nothing() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    take(&mut s, account, None, vec![window("Min", vec![mail("E1", "<1@x>", "One", 100), mail("E2", "<2@x>", "Two", 200)], true)], true);
    let answer = take(&mut s, account, None, vec![window("Min", vec![mail("E9", "<9@x>", "Nine", 900)], false)], true);
    assert_eq!((answer["new"].as_u64(), answer["removedMessages"].as_u64()), (Some(1), Some(0)));
    assert_eq!(in_folder(&s, account, "Min").len(), 3);
}

#[test]
fn what_was_changed_here_is_not_undone_by_the_server() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    take(&mut s, account, None, vec![window("Min", vec![mail("E1", "<1@x>", "One", 100)], true)], true);
    let id = in_folder(&s, account, "Min")[0].id;
    // Read and flagged here; the server hasn't heard of it yet.
    {
        let m = s.mailbox.message_mut(id).unwrap();
        m.read = true;
        m.flag = FlagStatus::Flagged;
    }
    // The server still says unread, unflagged, and now answered (from another client).
    let mut again = mail("E1", "<1@x>", "One", 100);
    again.server.answered = true;
    take(&mut s, account, None, vec![window("Min", vec![again], true)], true);
    let m = s.mailbox.message(id).unwrap();
    assert_eq!((m.read, m.flag, m.answered), (true, FlagStatus::Flagged, true));
    // Once the server says what was done here, the next change there is taken.
    let mut read_there = mail("E1", "<1@x>", "One", 100);
    (read_there.server.read, read_there.server.flag) = (true, FlagStatus::Flagged);
    take(&mut s, account, None, vec![window("Min", vec![read_there], true)], true);
    let mut unread_there = mail("E1", "<1@x>", "One", 100);
    unread_there.server.flag = FlagStatus::Flagged;
    take(&mut s, account, None, vec![window("Min", vec![unread_there], true)], true);
    assert!(!s.mailbox.message(id).unwrap().read);
}

#[test]
fn a_message_the_server_names_anew_is_the_same_message() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    take(&mut s, account, None, vec![window("Min", vec![mail("E1", "<1@x>", "One", 100), mail("E2", "<2@x>", "Two", 200)], true)], true);
    let one = in_folder(&s, account, "Min").iter().find(|m| m.subject == "One").map(|m| m.id).unwrap();
    // The server names every message anew (its ids say where a message is): the same messages,
    // taken back by their Message-ID, nothing new and nothing removed.
    let answer = take(&mut s, account, None, vec![window("Min", vec![mail("F1", "<1@x>", "One", 100), mail("F2", "<2@x>", "Two", 200)], true)], true);
    assert_eq!((answer["new"].as_u64(), answer["removedMessages"].as_u64()), (Some(0), Some(0)), "{answer}");
    assert_eq!(server_id(s.mailbox.message(one).unwrap()), "F1");
    assert_eq!(in_folder(&s, account, "Min").len(), 2);
}

#[test]
fn two_messages_that_could_be_the_one_listed_make_it_a_new_one() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    // Two copies of one message (the same Message-ID) in the Inbox.
    take(&mut s, account, None, vec![window("Min", vec![mail("E1", "<same@x>", "A", 100), mail("E2", "<same@x>", "B", 200)], true)], true);
    let answer = take(&mut s, account, None, vec![window("Min", vec![mail("F1", "<same@x>", "C", 300)], true)], true);
    // Neither is taken for it: a new message, and both copies go at the end of the run.
    assert_eq!((answer["new"].as_u64(), answer["removedMessages"].as_u64()), (Some(1), Some(2)), "{answer}");
    assert_eq!(in_folder(&s, account, "Min").iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["C"]);
}

#[test]
fn an_answer_for_another_account_or_nonsense_is_refused_or_passed_over() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    // For another address: refused, nothing changes.
    let stranger = JmapMailResult {
        account,
        email: "eve@example.org".into(),
        listing: Some(FolderListing { folders: vec![], complete: true }),
        ..Default::default()
    };
    assert!(apply_jmap_mail(&mut s, stranger).is_err());
    assert!(s.mailbox.folder_by_server(account, "Mpr").is_some());
    // A window for a mailbox it doesn't have, ids no server gives, and a message without an id.
    let pages = vec![
        window("Mnope", vec![mail("E1", "<1@x>", "One", 100)], true),
        window("bad id", vec![mail("E2", "<2@x>", "Two", 200)], true),
        window("Min", vec![mail("", "<3@x>", "Three", 300), mail("a/b", "<4@x>", "Four", 400), mail("E5", "<5@x>", "Five", 500)], false),
    ];
    let answer = take(&mut s, account, None, pages, false);
    assert_eq!(answer["new"], 1, "{answer}");
    assert_eq!(in_folder(&s, account, "Min").iter().map(|m| server_id(m)).collect::<Vec<_>>(), ["E5"]);
    // A result for an account that isn't kept in step with JMAP.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap();
    let wrong = JmapMailResult { account: imap["id"].as_u64().unwrap(), ..Default::default() };
    assert!(apply_jmap_mail(&mut s, wrong).is_err());
}

#[test]
fn an_answer_from_the_server_the_account_named_before_is_refused() {
    let (mut s, account) = session();
    let listing = Some(FolderListing { folders: mailboxes(), complete: true });
    // The run read SESSION; meanwhile the account was pointed at another server.
    s.run("account.edit", &json!({"id": account, "jmap": {"session": "https://other.example.org/jmap/session"}})).unwrap();
    let why = apply_jmap_mail(&mut s, JmapMailResult { session: SESSION.into(), listing: listing.clone(), ..piece(account) }).unwrap_err();
    assert_eq!(why.to_string(), format!("this Send/Receive was for the server {ME} named before"));
    assert!(!s.mailbox.folders.iter().any(|f| f.account == account && f.server.is_some()));
    // From the server it names now, spaces around the address or not, and from a host that doesn't say.
    let now = " https://other.example.org/jmap/session ";
    apply_jmap_mail(&mut s, JmapMailResult { session: now.into(), listing: listing.clone(), ..piece(account) }).unwrap();
    apply_jmap_mail(&mut s, JmapMailResult { listing, ..piece(account) }).unwrap();
    assert!(s.mailbox.folders.iter().any(|f| f.account == account && f.server.is_some()));
    // Nor are messages fetched whole from the server it named before.
    let fetched = vec![GraphFetched { id: "E1".into(), mime: b"Subject: Then\r\n\r\nFrom the other server.\r\n".to_vec() }];
    let late = GraphFetchResult { account, email: ME.into(), session: SESSION.into(), fetched, ..Default::default() };
    let why = apply_jmap_fetch(&mut s, late).unwrap_err();
    assert_eq!(why.to_string(), format!("this Send/Receive was for the server {ME} named before"));
}

#[test]
fn the_status_says_how_the_run_ended() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![], true);
    let answer = apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), auth_refused: true, ..Default::default() }).unwrap();
    assert_eq!(answer["authRefused"], true);
    assert!(s.sync_status.starts_with("The server refused the sign-in"), "{}", s.sync_status);
    let errors = (0..80).map(|n| format!("line {n}\nwith\u{7} control")).collect();
    let answer = apply_jmap_mail(&mut s, JmapMailResult { account, email: ME.into(), errors, ..Default::default() }).unwrap();
    let said = answer["errors"].as_array().unwrap();
    assert_eq!(said.len(), MAX_ERRORS);
    assert!(said.iter().all(|e| !e.as_str().unwrap().chars().any(char::is_control)));
}

#[test]
fn a_jmap_account_counts_among_those_with_a_server_and_sync_request_hands_it_its_own_request() {
    let (mut s, account) = session();
    // Only a JMAP account here: Send/Receive is there for it.
    assert!(s.enabled("sync.request"));
    let answer = s.run("sync.request", &json!({})).unwrap();
    assert_eq!(
        (answer["jmap"].as_bool(), answer["account"].as_u64(), answer["session"].as_str(), answer["window"].as_u64()),
        (Some(true), Some(account), Some(SESSION), Some(500))
    );
    assert!(answer.get("outbox").is_none() && answer.get("folders").is_none(), "not an IMAP request: {answer}");
    // Named, it is the same; an account of another kind is still handed its own.
    assert_eq!(s.run("sync.request", &json!({"account": account})).unwrap()["jmap"], true);
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap();
    let answer = s.run("sync.request", &json!({"account": imap["id"]})).unwrap();
    assert!(answer.get("jmap").is_none() && answer.get("outbox").is_some(), "{answer}");
    // Without an account of any kind with a server, it isn't there.
    let mut local = Session::empty();
    assert!(!local.enabled("sync.request"));
    assert!(local.run("sync.request", &json!({})).is_err());
}

#[test]
fn mail_written_in_a_jmap_account_waits_in_its_outbox_and_a_draft_waits_to_be_filed() {
    let (mut s, account) = session();
    let outbox = s.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap();
    let (sent, drafts) =
        (s.mailbox.folder_of_kind(account, FolderKind::Sent).unwrap(), s.mailbox.folder_of_kind(account, FolderKind::Drafts).unwrap());
    let count = |s: &Session, folder: Id| s.mailbox.messages_in(folder).count();
    s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Hello", "body": "Hi", "account": account})).unwrap();
    // Send queues it for the next Send/Receive, as for any account with a server; nothing is filed as sent here.
    let queued = s.run("compose.send", &json!({})).unwrap();
    assert_eq!(queued["queued"], true);
    assert!(s.compose.is_none());
    assert_eq!((count(&s, outbox), count(&s, sent), count(&s, drafts)), (1, 0, 0));
    // The run says what to send: the message whole, once.
    let request = jmap_mail_request(&s, account, false, &[], false).unwrap();
    assert_eq!(request.sends.len(), 1);
    assert_eq!((request.sends[0].mime.subject.as_str(), request.sends[0].mime.account), ("Hello", account));
    assert!(request.drafts.is_empty());

    // Save Draft keeps it in the account's Drafts and marks it to go up to the server's.
    s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Later", "body": "Hi", "account": account})).unwrap();
    let saved = s.run("compose.saveDraft", &json!({})).unwrap()["draft"].as_u64().unwrap();
    let draft = s.mailbox.message(saved).unwrap();
    assert_eq!((draft.folder, draft.draft, draft.server.is_none()), (drafts, true, true));
    let waiting = draft.server_draft.as_ref().unwrap();
    assert_eq!((waiting.pending, waiting.saves), (true, 1));
    assert_eq!((count(&s, outbox), count(&s, sent), count(&s, drafts)), (1, 0, 1));
    assert_eq!(super::jmap_send::pending_drafts(&s.mailbox, account), 1);
    assert_eq!(request_of(&s, account).drafts.iter().map(|d| (d.message, d.saves)).collect::<Vec<_>>(), [(saved, 1)]);
    // Every other kind of account still sends too: an IMAP account queues the message.
    let imap = s.run("account.add", &json!({"email": "bob@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    s.run("compose.discard", &json!({})).unwrap();
    s.run("mail.new", &json!({"to": "ada@example.org", "subject": "Hi", "account": imap})).unwrap();
    assert_eq!(s.run("compose.send", &json!({})).unwrap()["queued"], true);
}

/// What a run of `account` is asked to do.
fn request_of(s: &Session, account: Id) -> JmapMailRequest {
    jmap_mail_request(s, account, false, &[], false).unwrap()
}

#[test]
fn a_meeting_is_mailed_through_a_jmap_account_like_through_any_other() {
    use mailcraft_model::Attachment;
    const NOW: i64 = 1_791_993_600;
    // An item kept on this computer is forwarded as mail through an account with a mail server to send it.
    let mut s = Session::at(crate::sample::mailbox(NOW, 0), NOW, 0);
    // A host that runs a JMAP account's Send/Receive (this fork's browser build).
    s.jmap_runs = true;
    let review = s.mailbox.events.values().find(|e| e.title.starts_with("Design review")).map(|e| e.id).unwrap();
    let jmap =
        s.run("account.add", &json!({"email": ME, "kind": "jmap", "jmap": {"session": SESSION, "bearer": true}})).unwrap()["id"].as_u64().unwrap();
    let outbox = s.mailbox.folder_of_kind(jmap, FolderKind::Outbox).unwrap();
    s.run("calendar.forward", &json!({"id": review, "to": "dana@example.org", "send": true})).unwrap();
    assert_eq!(s.mailbox.messages_in(outbox).count(), 1, "the only account with a mail server to send it through");

    // An invitation in its Inbox is answered by mail from the account it came to.
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:plan-1\r\nSUMMARY:Planning\r\nSEQUENCE:0\r\nORGANIZER;CN=Bo Marsh:mailto:bo@example.org\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Ada:mailto:{ME}\r\nDTSTART:20261015T160000Z\r\nDTEND:20261015T170000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let inbox = s.mailbox.folder_of_kind(jmap, FolderKind::Inbox).unwrap();
    let invitation = Message {
        account: jmap,
        folder: inbox,
        message_id: "plan-1@example.org".into(),
        from: Address { name: "Bo Marsh".into(), email: "bo@example.org".into() },
        to: vec![Address { name: "Ada".into(), email: ME.into() }],
        subject: "Planning".into(),
        attachments: vec![Attachment::new("invite.ics", "text/calendar", ics.into_bytes())],
        ..Default::default()
    };
    let message = s.mailbox.add_message(invitation).unwrap();
    let said = s.run("calendar.invitation", &json!({"message": message})).unwrap();
    assert_eq!((said["answerable"].as_bool(), said["canSend"].as_bool()), (Some(true), Some(true)), "{said}");
    s.run("calendar.respond", &json!({"message": message, "response": "accept", "send": true})).unwrap();
    let waiting: Vec<&Message> = s.mailbox.messages_in(outbox).map(|m| m.as_ref()).collect();
    assert_eq!(waiting.len(), 2, "the forward and the answer wait in the account's Outbox for the next run");
    let answer = waiting.iter().find(|m| m.to.first().is_some_and(|a| a.email == "bo@example.org")).expect("the answer to the organizer");
    // ... and the run sends them.
    assert_eq!(request_of(&s, jmap).sends.len(), 2);
    assert!(request_of(&s, jmap).sends.iter().any(|send| send.message == answer.id));
}

#[test]
fn emails_read_by_their_ids_are_taken_in_as_no_send_receive_takes_them() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![window("Min", vec![mail("E1", "<1@example.org>", "One", 100)], true)], true);
    let status = s.sync_status.clone();
    // A message of Apollo that no window brought, read by its id: the last piece of its run, said whole.
    let read = JmapMailResult {
        account,
        email: ME.into(),
        pages: vec![window("Map", vec![mail("E9", "<9@example.org>", "Old plans", 50)], false)],
        whole: true,
        ..Default::default()
    };
    assert!(apply_jmap_opened(&mut s, &read).unwrap().is_empty(), "every page taken in");
    assert_eq!(in_folder(&s, account, "Map").iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Old plans"]);
    // No Send/Receive: its status stays, and nothing the page didn't list is taken for gone.
    assert_eq!(s.sync_status, status);
    assert!(in_folder(&s, account, "Min").iter().all(|m| !m.server.as_ref().is_some_and(|server| server.gone)));
    // A page that would be the whole of its mailbox is not taken; an answer for another account is refused.
    let whole = JmapMailResult { account, email: ME.into(), pages: vec![window("Min", Vec::new(), true)], ..Default::default() };
    apply_jmap_opened(&mut s, &whole).unwrap();
    assert!(in_folder(&s, account, "Min").iter().all(|m| !m.server.as_ref().is_some_and(|server| server.gone)));
    assert_eq!(in_folder(&s, account, "Min").len(), 1);
    assert!(apply_jmap_opened(&mut s, &JmapMailResult { account, email: "eve@example.org".into(), ..read.clone() }).is_err());
}

#[test]
fn an_email_read_by_its_id_never_takes_the_place_of_what_a_send_receive_read_since() {
    let (mut s, account) = session();
    take(&mut s, account, Some(server()), vec![window("Min", Vec::new(), true)], true);
    // A Send/Receive brought it meanwhile, read on the server.
    let mut newer = mail("E9", "<9@example.org>", "Old plans", 50);
    newer.server.read = true;
    take(&mut s, account, None, vec![window("Map", vec![newer], false)], false);
    // The answer to the reading by its id, older, says it is unread: it is not taken.
    let older = JmapMailResult {
        account,
        email: ME.into(),
        pages: vec![window("Map", vec![mail("E9", "<9@example.org>", "Old plans", 50)], false)],
        ..Default::default()
    };
    assert!(apply_jmap_opened(&mut s, &older).unwrap().is_empty());
    let here = in_folder(&s, account, "Map");
    assert_eq!(here.len(), 1);
    assert!(here[0].read, "what the Send/Receive read stays");
}

#[test]
fn an_email_read_by_its_id_before_the_folders_are_known_waits_for_them_or_comes_with_its_listing() {
    // A tab that has just opened: no Send/Receive has listed the mailboxes yet.
    let (mut s, account) = session();
    let page = window("Map", vec![mail("E9", "<9@example.org>", "Old plans", 50)], false);
    let alone = JmapMailResult { account, email: ME.into(), pages: vec![page.clone()], ..Default::default() };
    // Its folder isn't here: the page is handed back, and nothing is taken in.
    assert_eq!(apply_jmap_opened(&mut s, &alone).unwrap(), vec![page.clone()]);
    assert!(s.mailbox.messages.values().all(|m| m.subject != "Old plans"));
    // Once a Send/Receive has listed them, the page handed in again is taken.
    take(&mut s, account, Some(server()), Vec::new(), false);
    assert!(apply_jmap_opened(&mut s, &alone).unwrap().is_empty());
    assert_eq!(in_folder(&s, account, "Map").len(), 1);

    // A run by id that brings the listing itself (the folders weren't known when it was asked for).
    let (mut s, account) = session();
    let listed = JmapMailResult {
        account,
        email: ME.into(),
        listing: Some(FolderListing { folders: server(), complete: true }),
        pages: vec![page],
        ..Default::default()
    };
    assert!(apply_jmap_opened(&mut s, &listed).unwrap().is_empty());
    assert_eq!(in_folder(&s, account, "Map").iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Old plans"]);
}
