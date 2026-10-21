<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

This is Neoffice's fork of [UCS-Care-Group/mailcraft](https://github.com/UCS-Care-Group/mailcraft). Its only branch,
`version-15`, is what Neoffice publishes. Every change to upstream's files carries a comment that starts with four slashes
after the language's comment opener and says why (`// //// Neoffice — …` in Rust, `# //// Neoffice — …` in TOML and YAML,
`<!-- //// Neoffice — … -->` in Markdown), so that `grep -rn "////"` maps the whole divergence at the next upstream merge.
A file this fork adds says so in its first lines. `.github/workflows/neoffice-markers.yml` checks it on every push.

## Base of the divergence

- `8ec913a` (upstream `main` on 2026-10-21 at noon: 0.6.0; the interface in French, German, Spanish and Italian by
  upstream's own catalogues, the crate `mailcraft-i18n` with its `.po` files in `crates/i18n/po/`, the `tl!`, `tlc!`
  and `tln!` macros, `cargo xtask i18n` and its `--check`, the dates written by `crates/ui-egui/src/dates.rs` and the
  language followed by the week and the clock (`MailApp::apply_language`, `Services::system_locale`, #255, #258, #259,
  #260, #263); the engine's refusals said in the interface's language (`CmdError::Said`); inline widths and centring
  in a message's HTML (#275); Calendar › View with the four views (#239); signature pictures over the cap made smaller
  (#270); the appointment's window in dark mode (#266); the rest of a new shared mailbox no longer holding up the
  owner's (#70)), merged into `version-15` on 2026-10-21. The fork's own interface catalogue (`fr.tsv`, its lookup in
  `crates/ui-egui/src/i18n/catalog.rs` and `interface_strings.rs`, its Options row "Language" and its `ui.language`
  command) gave way to upstream's: there is one Language row and one `ui.language`, upstream's; the fork's texts are
  written with upstream's macros and extracted by upstream's `cargo xtask i18n`, so `messages.pot` and the four
  catalogues it writes hold them too (`crates/i18n/po/`, listed below), and their French is in the fork's own
  catalogue `crates/i18n/po/neoffice/fr.po`, read over upstream's French (`crates/i18n/build.rs`, `OVERLAYS`;
  `Catalogue::overlaid` in `crates/i18n/src/lib.rs`). A text both have keeps upstream's French; the fork's catalogue
  holds the fork's own texts, the texts the window looks up from a table of its own that upstream's catalogue lacks,
  and one choice of the fork's: a numeric date day first with dots, as in Switzerland (`21.10.2026`, its entry in
  that catalogue and the marked line in `crates/i18n/src/date.rs`). The fork's `DateWords` and `QuoteWords` gave way
  to upstream's `dates` and catalogues (the header over a quoted original, the calendar's titles); 24-hour time and
  the week from Monday follow the language as upstream has it. The fork's tests now say what upstream's engine says
  (`CmdError::Said` where they expected `CmdError::Failed`).
- Before it: `3f9cf26` (upstream `main` on 2026-10-20 in the evening: a JMAP account in the model, M6, upstream's own shape of what
  this fork had there (`AccountKind::Jmap`, `JmapServer`, `sync/jmap.rs`, the answered and forwarded marks of a
  `ServerMessage`); an account of a kind this version doesn't know kept as its file has it, `AccountKind::Unknown`; shared
  mailboxes out of testing, with moves and copies between a shared mailbox and its owner's mailbox and File › Signatures
  listing them, #70; reminder windows on the main window's theme; a CodeQL workflow; README and roadmap documents), merged
  into `version-15`. Where upstream now has what the fork had, the fork takes upstream's text and keeps only what it adds,
  marked: `crates/model/src/jmap_server.rs` is upstream's whole, and `crates/model/src/sync/jmap.rs` is upstream's with the
  sending and the drafts of a JMAP account on top (`JmapSend`, `JmapDraftUp`, `JmapDraftDone`, `unwanted`); neither is the
  fork's own file any more. Upstream's M6 guards ("nothing is sent from a JMAP account yet", "no Send/Receive is made of a
  JMAP account until the engine has its commands") give way where this fork has those commands: `AccountKind::sends_mail`
  is true for a JMAP account, `sync.request` hands a JMAP account to its JMAP run (`sync.jmapRequest`), the window takes a
  JMAP account into its Send/Receive where the host gives it a JMAP runner (`Services::can_run`), and upstream's tests of
  those guards (`crates/engine/src/cmd/jmap_sending_tests.rs`, `cmd/calendar/invitation.rs`, `crates/ui-egui/src/tests.rs`)
  say what this fork does instead. The guards themselves stay, for a build or a caller that can't run one (the command
  line's `sync`, `sync_request`).
- Before it: `73cbee4` (upstream `main` on 2026-10-20 in the morning: SPDX lines in every Rust file and the `cargo xtask spdx`
  check, one pinned Rust version for CI, the Scorecard and reproducible-build workflows, and the two roadmap documents); the
  fork's own Rust files got their SPDX lines by `cargo xtask spdx --fix` in the commit after that merge. Then `d7db890`
  (0.5.2, the desktop alerts, the project's new Microsoft registration, About ▸ Testers, and the governance documents),
  `ed2bd41` (upstream `main` on 2026-10-19 in the evening: 0.5.1, the security review fixes and the fuzz targets), and
  `d33985a` (upstream `main` on 2026-10-19: v0.5.0 and the contributor licence agreement), where the fork was created.

## What the fork adds

- `.github/workflows/neoffice-markers.yml`: the marker check above.
- `.github/workflows/neoffice-ci.yml`: upstream's gate (`cargo xtask ci`, then `cargo xtask deny`) in one job on a GitHub
  runner, and a `cargo check` of the web app for wasm32, which `cargo xtask wasm` leaves out; upstream's own workflows are
  disabled on this fork.
- `.github/workflows/neoffice-web.yml`: the browser build by upstream's packaging script, kept as an artifact or published
  as a release (`web-<version>-<short sha>`).
- `crates/jmap/` (`mailcraft-jmap`, layer 3): JMAP (RFC 8620 and 8621) without sockets or browser APIs, so that mail can be
  read over HTTPS in the browser, where IMAP and SMTP can't go. An async client over an `Http` trait, the address helpers,
  `sync_mail` (an account's mailboxes and the newest messages of each, for the engine, after the changes made
  here have been told to the server with `Email/set`), the raw octets of a message, fetched by its email id
  (`fetch`), the sending of what waits in an account's Outbox and the filing of its drafts on the server (`send`,
  with a run's part of it in `mail/outgoing`), and an in-process server for the tests of the crates that sit on it
  (the `stand-in` feature). A crate with no upstream equivalent, written so that it can be offered upstream: nothing
  in it names this fork's products. Its URL and HTTP helpers (`src/url.rs`, `src/http.rs`) and the browser build's
  `fetch` transport (`apps/mailcraft-web/src/fetch.rs`, `fetch_rules.rs`) are written fresh from RFC 3986, 6570, 7617,
  6750, 8620 and 9110 and the WHATWG URL and Fetch Standards.
- Nora, the assistant of the Neoffice server, when a page that hosts the browser build reaches her (nothing of her
  anywhere else): `crates/ui-egui/src/nora.rs` and `nora/` (her answers read as a stranger's bytes, what she is given,
  seven window commands `ui.nora*` that agents may never run, her band under the subject of a message being written,
  her button in the reading pane and her translation in place of a message's body, her place in the ribbon), and
  the browser's way to her in `apps/mailcraft-web/src/nora.rs` and `nora_origin.rs`. The marked hunks are in
  `crates/ui-egui/src/lib.rs` (the module, the host's field of `Services`, the commands, the window's state, the poll
  once a frame), `reading.rs`, `compose.rs`, `ribbon.rs`, `icons.rs`, `crates/engine/src/agent.rs`,
  `apps/mailcraft-web/src/main.rs` and `web.rs`; her French lines are in `crates/i18n/po/neoffice/fr.po`. The rules:
  only the six methods Neodesk's shell lets through are called; a message of a mailbox the page handed over goes by
  its id on the server, never its text, the sample mailbox's by its text, any other mailbox not at all; a mailbox of
  the page is named by the name of the server's account for it (the page's `account`, else the `account` of its
  session's address, else its address), which is not its address once that changed; the origin of
  her calls never comes from the address, but for a loopback page naming a loopback origin (the workshop's stand-in
  server), and the page's CSRF token goes to the page's own origin only; a hosting page that makes the calls itself
  offers `window.mailcraftNoraCall`; every call ends (195 s in the browser, 200 s in the window); what she writes
  goes into the composer as one step Undo takes back, and nothing she writes is sent.
- `apps/mailcraft-cli/tests/jmap_list.rs`, `jmap_fetch.rs`, `jmap_up.rs` and `jmap_send.rs`: the engine, `sync_mail`
  and `fetch` against that in-process server, end to end; the last holds the sending of a message and the filing of a
  draft to both kinds of server the stand-in can be.
- The sentences a JMAP Send/Receive and the engine write in English, read back in the interface's language where they
  are drawn (the status bar and its tooltip, Send/Receive Progress and Copy problems, the banner of a message held in
  the Outbox, a command's refusal, what Undo couldn't take back): `crates/ui-egui/src/i18n/run_words.rs` and
  `run_words/` (the matcher, the table of the crates' sentences, written with `run_word!`, which upstream's extractor
  takes as it takes `tl!` (a marked branch of `xtask/src/i18n.rs`), so that `messages.pot` and every catalogue have
  them, and its tests, among them a census of the files that write them, `run_words/census.rs`). The crates and the
  engine are not changed: what they write stays English in the mailbox file, the log and the control channel; a JMAP
  run's progress words are asked for in English (`crates/engine/src/cmd/jmap_mail.rs`). The marked hunks are in
  `crates/ui-egui/src/i18n.rs` (`t_in`, the module, the window's language kept per thread in the tests), `lib.rs`
  (`run_word!`, Copy problems, the refusals of `run`: a refusal the catalogue doesn't have whole is read by
  `run_words`), `chrome.rs` (the status), `sync_panel.rs` (the lines of the panel), `reading.rs` (the banner) and
  `host_menu.rs` (a command's refusal). Their French lines are in `crates/i18n/po/neoffice/fr.po`. The guard that makes
  the real runner fail over the in-process server, in every way it and a stand-in transport can, and reads every line
  in French is `apps/mailcraft-web/src/run_words_tests.rs`.
- The name of a server's folder of a standard kind in English too: "Inbox" for a server's "INBOX", "Sent Items" for its
  "Sent", as this fork's builds have always shown them (upstream shows the server's name in English and translates it
  in the other languages): a marked hunk of `folder_name` in `crates/ui-egui/src/nav.rs`, with `named_kind`, the kind
  `run_words` names a folder by. A hosting page that draws the folders in its own menu is told the same names.
- A hosting page's language first: the browser build takes the language the hosting page says the person reads
  (`crates/ui-egui`'s `Services::system_locale`, from `apps/mailcraft-web/src/host.rs`) before the browser's (`web.rs`).
- An online meeting in a JMAP account's calendar, made by its server (a room of its own, asked for by a virtual
  location whose address is `urn:x-online-meeting:new`, its link read back from the event's `virtualLocations`): the
  reading and the writing are in `crates/jmap/` (`pim/jscalendar.rs`, `pim/calendars.rs`, `pim/write.rs`,
  `pim/push.rs`, and the stand-in's rooms in `standin/pim.rs` and `standin/pim/set.rs`); the marked hunks are in
  `crates/model/src/calendar.rs` (`Calendar::online_service`: Teams where allowed, else the server's own), the engine's
  `cmd/calendar.rs` (a plain calendar refuses one only where its server makes none, and a meeting made online here takes
  its calendar's service), `cmd/calendar/meeting.rs` (`teams_refusal`, `online_fits`, `online_here`, the words of an
  online meeting that is no Teams one), `cmd/remote.rs` (what became of one, said as an online meeting, also for one
  occurrence), `crates/ui-egui/src/forms.rs` (the toggle and the row say "Online Meeting") and `calendar_view.rs`
  (such a meeting is joined by "Join Online Meeting"); its French lines are in `crates/i18n/po/neoffice/fr.po`, and the
  engine's sentences have their rows in `crates/ui-egui/src/i18n/run_words/rows.rs`.
- Notes, the mailbox's own and those of a server that keeps them (the hosting page's server, whose notes are the
  person's: one place for notes): the model (`crates/model/src/note.rs`), the commands, their refusals and what an agent
  may do (`crates/engine/src/cmd/notes.rs` and `notes/`, among them `notes/server.rs`, what is noted for the server and
  taken back from it), the job's part (`crates/jmap/src/pim/notes.rs`, its capability's name the one place the crate
  names that server) and the stand-in's notes (`crates/jmap/src/standin/notes.rs`), the window's job
  (`crates/ui-egui/src/jmap_notes.rs`), the module (`notes_view.rs`, `ribbon/notes_home.rs`), the browser's replay
  of a job done again (`apps/mailcraft-web/src/jmap_notes.rs`) and its question before a hosting page is left
  (`hosted/leave.rs`), and their tests (`apps/mailcraft-cli/tests/jmap_notes.rs`, `apps/mailcraft-web/src/jmap_notes_flow_tests.rs`).
  The marked hunks are in `crates/model/src/lib.rs` and `mailbox.rs` (the field, its repair and the accounts that go),
  `crates/engine/src/session.rs` (`Module::Notes`, the selected note), `cmd/mod.rs`, `agent.rs` (the notes' classes,
  `ui.openNote` among them), `crates/ui-egui/src/lib.rs` (the module, `ui.openNote`), `nav.rs` (Notes in the module
  bar where notes are kept, and each module's button in the hit map), `ribbon.rs`, `chrome.rs`, `keys.rs`,
  `apps/mailcraft-web/src/web.rs` and `Cargo.toml` (the `web-sys` features of the question); their French lines are in
  `crates/i18n/po/neoffice/fr.po`.
- A hosting page's tab, which keeps nothing of its own between visits (`Session::keeps_local` false), refuses what it
  would lose, with the reason a greyed control says: Tasks are not offered at all (`view.tasks`, `tasks.new`, Add to
  Tasks, Mod+4, the module bar), nor Import, nor a contact's favourite and categories
  (`crates/engine/src/cmd/kept_here.rs`; File › Open & Export greys its Import tile, `backstage.rs`); New Contact says
  whether the books are still being read (`Session::books_listed`), none is listed or the rights forbid it, and Delete
  says it is deleting that the rights forbid (`cmd/remote_contacts.rs`). A calendar whose server keeps no categories,
  importance or other times proposed, and asks everyone invited for an answer (`Calendar::plain`: this program writes
  every person invited as asked to answer), greys them and the response options in the ribbon, the Categorize menus,
  the appointment's window and the invitation bar, with one reason (`cmd/calendar.rs` `needs_kept_options`,
  `options_refusal`; `forms.rs`, `ribbon.rs`, `reading/invitation.rs`); an appointment moved into one in its form
  drops them, and it refuses to forward in words of its own. Delete is greyed for an event of a calendar the account
  may only read (`needs_changeable_event`). Such a tab asks before a server's event with nobody to tell is deleted, or
  one day of its series, naming the calendar, and before a server's contact is (an administrator restores it from the
  instance's Deleted Documents), in `crates/ui-egui/src/dialogs.rs`; it keeps which of the servers' calendars are
  ticked with the interface's settings, by the account's address and the calendar's name on its server, noted whenever
  the person changes them and put back on the calendars the first list of a visit makes
  (`crates/ui-egui/src/calendar_ticks.rs`, `UiState::calendar_ticks`). The window's tests of all this are
  `crates/ui-egui/src/kept_here_tests.rs`.
- The hosted browser build tells the page that frames it whether closing it now would lose work: a message being
  written with changes, an appointment's window or a form a dialog holds with something typed into it (an
  appointment's, a contact's), and in a tab that keeps nothing, a change of an event or a contact not told to its
  server yet, mail in an Outbox, a draft saved here that the next run files, a move or a delete for good not told yet,
  a JMAP account's messages read, flagged or moved here (worked out once for each state of the mailbox), or what the
  notes' own question counts (`MailApp::notes_unsent`): `MailApp::unsent_work`, `crates/ui-egui/src/unsent.rs`. It speaks the Craft
  editors' contract with their host page (`<editor>:dirty`), in `apps/mailcraft-web/src/host_dirty.rs`, sent from the
  shell's frame loop (`apps/mailcraft-web/src/web.rs`, a tab of a hosting page only):
  `window.parent.postMessage({type: "mailcraft:dirty", dirty: true|false}, location.origin)` — once as the tab starts
  (`false`, or `true` if it already holds such work), then at each change and never twice the same; nothing when the
  page is not framed. **What the host must do** (Neodesk, `desktop/src/editors/host/host.js`, in `startMail`, not done
  yet): listen to `message` on the host page's window, take an event only when `event.source === frame.contentWindow`
  and `event.origin === location.origin` and `event.data` is an object whose `type` is `"mailcraft:dirty"`, and call
  `bridge.setDirty(Boolean(event.data.dirty))` — the `editors:dirty` message that `index.js` already turns into
  `session.dirty`, which its `onClose` asks about. NeoMail has no Save for that question to run
  (`saveBeforeClosing` drives an editor's Save): for a window of `kind === 'mail'` it offers leaving and staying only.
  The frame stays loaded as now (no `?host=parent`); the browser's own `beforeunload` question (`hosted/leave.rs`, the
  notes') is left as it is.
- A hosting page that draws MailCraft's navigation in its own menu (Neoffice's desk at `/app/neomail`, which hands
  `chrome: "menu"` with its mailboxes; Neodesk never does, and without it nothing changes): the window draws no module
  bar and, in Mail, no folder pane, not even its strip, so that the message list starts at its left edge; the ribbon
  with its search box, the list, the reading pane, the side panes of Calendar, People and Notes and every shortcut
  stay (`Services::host_menu`; the marked hunks are in `crates/ui-egui/src/lib.rs`, `mail_view.rs` and `nav.rs`, whose
  folder order and unread counts the page is told). What the page is told and what it may ask are
  `crates/ui-egui/src/host_menu.rs`, natively tested with their window in `host_menu/tests.rs`: posted to
  `window.parent` on the page's own origin, `{type: "mailcraft:ready", version}` once, then
  `{type: "mailcraft:state", module, account, folder, favorites, accounts}` at each change, at most every 250 ms and
  never twice the same, of the mailboxes the page handed over alone (their folders in the pane's order with the name
  shown, the role, the parent, the unread mail known here, the number the pane shows beside each (`count`: unread,
  or the total it puts in brackets for Drafts, Outbox and Junk Email, or null), by the pane's own rule, which
  `nav.rs` keeps in one place for both (`shown_count`), and the server's id, `local:<n>` for one only MailCraft has;
  their Favourites as the pane lists them; the mailbox and folder in view; the views offered, `notes` only where
  Notes are; no message, no other address, no token);
  and eight commands from `window.parent` on that origin (`event.source`, `event.origin`), any other ignored:
  `view.mail`, `view.calendar`, `view.people`, `view.notes`, `folder.select {account, folder}`, `mail.new`,
  `search {query}` (the search box's) and `message.open {account, id}` (a message by its email id on the server,
  shown as a click in the list shows it), each run as the window's own button or box runs it and answered
  `{type: "mailcraft:command-done", id, ok, error?}`, the reason in the interface's language; any other name, a
  parameter it doesn't take or an id that is no number nor a short text is refused. A message `message.open` names
  that no window has brought yet (a link from a notification) is read from its server by its id alone, and the
  answer waits for it, 30 seconds at most: `JmapMailRequest::emails` (a marked hunk of `crates/model/src/sync/jmap.rs`,
  ten at most), a run of `crates/jmap/src/mail.rs` that lists the mailboxes and reads those emails and nothing else,
  each into the mailbox whose window lists it, and `apply_jmap_opened` (`crates/engine/src/cmd/jmap_mail.rs`), which
  takes them in as no Send/Receive (the account's progress and status stay as they were, nothing is taken for gone). The browser's part (the flag,
  the listener, the posts, the throttle, the version with the commit the web build's workflow knows) is
  `apps/mailcraft-web/src/host.rs` (`menu`, and the contract in its comment), `host_menu.rs`, `main.rs`, `web.rs` and
  `Cargo.toml` (the `MessageEvent` feature of `web-sys`); the refusals' French lines are in
  `crates/i18n/po/neoffice/fr.po`.
- The engine's and the JMAP crate's sentences about a JMAP account's calendars and address books, read back in the
  interface's language by `run_words`: what a job says went wrong (`crates/jmap/src/pim.rs`, `pim/calendars.rs`,
  `pim/contacts.rs`), why a server didn't take a change (`pim/push.rs`, `pim/contacts/push.rs`, `pim/write.rs`), what
  became of a change of an event or a contact (`crates/engine/src/cmd/remote.rs`, `cmd/remote_contacts/written.rs`)
  and the engine's refusals of their items (`cmd/remote_contacts.rs`, `cmd/calendar.rs`, `cmd/kept_here.rs`,
  `crates/engine/src/lib.rs`'s "not available"). A line is read with the sentences the engine puts after it
  (`Family::Tail`), an item with no name by the interface's words for one, and what of an event a server shows
  otherwise by its parts (`Family::Parts`). The status bar reads its message through `run_words` too
  (`crates/ui-egui/src/chrome.rs`), and a change that waits says its reason in the interface's language
  (`calendar_sync.rs`, `jmap_pim.rs`); the status bar reads a message once for each text and language. Their French
  lines are in `crates/i18n/po/neoffice/fr.po`; the census of the tests holds those files to their rows.
- A company signature a hosting page hands over with a mailbox (`apps/mailcraft-web/src/page_signature.rs`, read in
  `host.rs`): its HTML or its text, then its pictures by their bytes (`pictures: [{data, name, alt, width, lead, href}]`,
  three at most), the first beside or above the words when `lead`, the others under them in paragraphs of their own, and
  a link for one that has an `href` (the company's banner), as the owner would put them in. With its pictures the page
  gives `placed_html`, its HTML without any of them nor a link they leave holding nothing, read in place of `html` when
  MailCraft takes some of them (`taken_html`): else the banner went out twice, its description as linked words above
  the picture; `html` stays for the builds that place one picture, where those words are what shows of it. A picture that is a link is
  written inside it in the sent HTML, under a text link's rule (`crates/mime/src/rich.rs`, `linked_img`, the picture
  beside or above the words included), and says where it leads in the text part (`crates/model/src/rich.rs`,
  `[alt] <url>`): upstream keeps the link in the model and dropped it there. In the dark theme the composer and
  File › Signatures stand a picture that is dark ink on nothing (a dark logo with a see-through background) on a light
  plate, on screen only (`crates/ui-egui/src/compose/editor.rs`, `plate`; `reading/images.rs`, `dark_on_clear`).
- A hosting page that takes a message to share in its team chat (Neoffice's desk at `/app/neomail`, which hands
  `share: true`; its chat is Synk, the instance's Raven): Share in Synk, a button in the Respond group of Mail's Home
  tab and an entry of a message's context menu after Forward, posts `{type: "mailcraft:share", account, email}` to
  `window.parent` on the page's own origin for the message in the reading pane, by its mailbox's address (as the page
  handed it over) and its id on the server alone, only on the person's click and only for a message of a mailbox the
  page handed over that its server has (`crates/ui-egui/src/host_share.rs`, the window command `ui.shareMessage`,
  tested with its window in `host_share/tests.rs`; `Services::share` and the command's hunks in `lib.rs`, the button's
  in `ribbon.rs`, the entry's in `mail_view.rs`, its drawing `shareChat` in `icons.rs`). The browser's part is
  `apps/mailcraft-web/src/host.rs` (`share`, and the contract in its comment), `host_menu.rs` (`tell`) and `web.rs`;
  the French lines are in `crates/i18n/po/neoffice/fr.po`. Without `share: true` nothing is offered.
  Beside it, Chat in Synk, a button of the same group with a menu (With the Sender, With Everyone; the window
  commands `ui.chatWithSender` and `ui.chatWithEveryone`, also two entries of the context menu after Share in Synk),
  posts `{type: "mailcraft:chat", account, email, who: "sender" | "all"}` the same way, for the same messages; the
  page finds or makes the conversation and opens it in Synk (`host_share.rs`, `chat` and `menu`; the menu's
  `Menu::HostChat` in `ribbon.rs`, its drawing `chatSynk` in `icons.rs`, the commands kept from agents in
  `crates/engine/src/agent.rs`, `NEVER`).
- This file.

## What cannot carry a comment

A `Cargo.lock` change or a binary asset that differs from upstream gets a line here, and so does any file of a format
the marker check does not know as one that takes comments:

- `Cargo.lock`: the `mailcraft-jmap` package (0.6.0, the workspace's version, with its dependencies on `mailcraft-mime`,
  `mailcraft-model` and `mailcraft-time`), the dependencies on it of the web app and, for its tests, the command line,
  and nothing else. (The newer `rustls-webpki` and `webpki-roots` this fork took for RUSTSEC-2026-0381 and 0382 came
  with upstream's 0.6.0 and are upstream's lines now.)
- `crates/i18n/po/messages.pot`, `crates/i18n/po/fr.po`, `crates/i18n/po/de.po`, `crates/i18n/po/es.po` and
  `crates/i18n/po/it.po`: written by `cargo xtask i18n`, never by hand. They differ from upstream's by the fork's
  texts the extractor finds (new entries, untranslated in upstream's catalogues: their translations are in the fork's
  catalogues below) and by the files named in the references of
  shared texts. At the next upstream merge: take upstream's, then run `cargo xtask i18n` again.
- `crates/i18n/po/neoffice/fr.po`: the fork's own French catalogue, read over upstream's (above). Its first line is a
  `#` comment that says it is ours (the format has comments), but the check does not read `.po` comments. A catalogue
  added next to it for another language (`crates/i18n/po/neoffice/<code>.po`, read by the same `OVERLAYS`) is named
  here the same way.
- `crates/i18n/po/neoffice/de.po`, `crates/i18n/po/neoffice/es.po` and `crates/i18n/po/neoffice/it.po`: the same
  entries as `fr.po`, in German, Spanish and Italian (translated by Haiku), read over upstream's catalogues the same
  way; `it.po` also writes a numeric date with dots, as in Switzerland (`crates/i18n/src/date.rs`, its test). When a
  text is added to `fr.po`, it is added to these three too.
