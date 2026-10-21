<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

This is Neoffice's fork of [UCS-Care-Group/mailcraft](https://github.com/UCS-Care-Group/mailcraft). Its only branch,
`version-15`, is what Neoffice publishes. Every change to upstream's files carries a comment that starts with four slashes
after the language's comment opener and says why (`// //// Neoffice — …` in Rust, `# //// Neoffice — …` in TOML and YAML,
`<!-- //// Neoffice — … -->` in Markdown), so that `grep -rn "////"` maps the whole divergence at the next upstream merge.
A file this fork adds says so in its first lines. `.github/workflows/neoffice-markers.yml` checks it on every push.

## Base of the divergence

- `3f9cf26` (upstream `main` on 2026-10-20 in the evening: a JMAP account in the model, M6, upstream's own shape of what
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
  in it names this fork's products.
- Nora, the assistant of the Neoffice server, when a page that hosts the browser build reaches her (nothing of her
  anywhere else): `crates/ui-egui/src/nora.rs` and `nora/` (her answers read as a stranger's bytes, what she is given,
  seven window commands `ui.nora*` that agents may never run, her band under the subject of a message being written,
  her button in the reading pane and her translation in place of a message's body, her place in the ribbon), and
  the browser's way to her in `apps/mailcraft-web/src/nora.rs` and `nora_origin.rs`. The marked hunks are in
  `crates/ui-egui/src/lib.rs` (the module, the host's field of `Services`, the commands, the window's state, the poll
  once a frame), `reading.rs`, `compose.rs`, `ribbon.rs`, `icons.rs`, `crates/engine/src/agent.rs`,
  `apps/mailcraft-web/src/main.rs` and `web.rs`; her French lines are a block near the top of `fr.tsv`. The rules:
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
  `run_words/` (the matcher, the table of the crates' sentences, written with `run_word!` so that every catalog must
  have them, and its tests, among them a census of the files that write them). The crates and the engine are not
  changed: what they write stays English in the mailbox file, the log and the control channel. The marked hunks are in
  `crates/ui-egui/src/i18n.rs` (`t_in`, a `fmt` that fills in one pass, the module), `lib.rs` (`run_word!`, Copy
  problems, the refusals of `run`), `chrome.rs` (the status), `sync_panel.rs` (the lines of the panel) and
  `reading.rs` (the banner); `i18n/interface_strings.rs`, the fork's own, reads `run_word!` as a lookup and hands the
  census the literals of a source file. Their French lines are a block near the top of `fr.tsv`. The guard that makes
  the real runner fail over the in-process server, in every way it and a stand-in transport can, and reads every line
  in French is `apps/mailcraft-web/src/run_words_tests.rs`.
- Three French defects of the window, fixed in upstream's files (marked): the header over the original a reply or a
  forward quotes is written in the interface's words (`QuoteWords` in `crates/engine/src/cmd/compose.rs`, which the
  window sets with its language in `crates/ui-egui/src/i18n.rs`; English, as before, for an agent or the command line),
  with its date as the window writes a message's; and in `crates/ui-egui/src/calendar_view.rs` the titles of the
  calendar's days and weeks are templates of the catalogs (« 18 – 24 octobre 2026 »), and the label of the row of
  whole-day events wraps in its gutter when a language's words are longer than "All day".
- An online meeting in a JMAP account's calendar, made by its server (a room of its own, asked for by a virtual
  location whose address is `urn:x-online-meeting:new`, its link read back from the event's `virtualLocations`): the
  reading and the writing are in `crates/jmap/` (`pim/jscalendar.rs`, `pim/calendars.rs`, `pim/write.rs`,
  `pim/push.rs`, and the stand-in's rooms in `standin/pim.rs` and `standin/pim/set.rs`); the marked hunks are in
  `crates/model/src/calendar.rs` (`Calendar::online_service`: Teams where allowed, else the server's own), the engine's
  `cmd/calendar.rs` (a plain calendar refuses one only where its server makes none, and a meeting made online here takes
  its calendar's service), `cmd/calendar/meeting.rs` (`teams_refusal`, `online_fits`, `online_here`, the words of an
  online meeting that is no Teams one), `cmd/remote.rs` (what became of one, said as an online meeting, also for one
  occurrence) and `crates/ui-egui/src/forms.rs` (the toggle and the row say "Online Meeting"); its French lines are in
  `fr.tsv`, and the engine's sentences have their rows in `crates/ui-egui/src/i18n/run_words/rows.rs`.
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
  `apps/mailcraft-web/src/web.rs` and `Cargo.toml` (the `web-sys` features of the question); their French lines are a
  block near the top of `fr.tsv`.
- A hosting page's tab, which keeps nothing of its own between visits (`Session::keeps_local` false), refuses what it
  would lose, with the reason a greyed control says: Tasks are not offered at all (`view.tasks`, `tasks.new`, Add to
  Tasks, Mod+4, the module bar), nor Import, nor a contact's favourite and categories
  (`crates/engine/src/cmd/kept_here.rs`); New Contact says whether no book is listed or the rights forbid it, and
  Delete says it is deleting that the rights forbid (`cmd/remote_contacts.rs`). A calendar whose server keeps no
  categories, importance, response options or other times proposed (`Calendar::plain`) greys them in the ribbon, the
  Categorize menus, the appointment's window and the invitation bar, with one reason (`cmd/calendar.rs`
  `needs_kept_options`, `options_refusal`; `forms.rs`, `ribbon.rs`, `reading/invitation.rs`), and refuses to forward
  in words of its own. Such a tab asks before a server's event with nobody to tell is deleted, and before a server's
  contact is (an administrator restores it from the instance's Deleted Documents), in `crates/ui-egui/src/dialogs.rs`;
  it keeps which of the servers' calendars are ticked with the interface's settings, by the account's address and the
  calendar's name on its server (`crates/ui-egui/src/calendar_ticks.rs`, `UiState::calendar_ticks`). The window's
  tests of all this are `crates/ui-egui/src/kept_here_tests.rs`.
- This file.

## What cannot carry a comment

A `Cargo.lock` change or a binary asset that differs from upstream gets a line here, and so does any file of a format
the marker check does not know as one that takes comments:

- `Cargo.lock`: the `mailcraft-jmap` package (with its dependencies on `mailcraft-mime`, `mailcraft-model` and
  `mailcraft-time`), the dependencies on it of the web app and, for its tests, the command line, and nothing else.
- `Cargo.lock`: `rustls-webpki` 0.103.16 (was 0.103.15) and `webpki-roots` 1.0.10 (was 1.0.9), for RUSTSEC-2026-0382
  and RUSTSEC-2026-0381 (distrusted emSign intermediates), which fail `cargo deny` since 2026-10-20. To drop when an
  upstream merge brings these versions or later.
- `crates/ui-egui/src/i18n/fr.tsv`: the French interface catalog. Its first line is a `#` comment that says it is ours
  (the format has comments), but the check reads a `.tsv` as text without comment syntax. Each catalog added next to
  it (`crates/ui-egui/src/i18n/<code>.tsv`) is named here the same way.
