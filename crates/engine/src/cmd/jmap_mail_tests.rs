// //// Neoffice — added file (no upstream equivalent)
//! A JMAP account as the engine works out what a run reads and takes in what comes back: the
//! folder list, complete and partial windows, the keywords, and messages whose id changed.

use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMessage, MessagePage, ServerMessage};
use mailcraft_model::{Address, FlagStatus, Message};
use serde_json::json;

use super::*;

const ME: &str = "ada@example.org";
const SESSION: &str = "https://mail.example.org/jmap/session";

/// A session with one JMAP account; its id.
fn session() -> (Session, Id) {
    let mut s = Session::empty();
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
