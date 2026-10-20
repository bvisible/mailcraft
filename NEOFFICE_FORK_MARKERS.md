<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

This is Neoffice's fork of [UCS-Care-Group/mailcraft](https://github.com/UCS-Care-Group/mailcraft). Its only branch,
`version-15`, is what Neoffice publishes. Every change to upstream's files carries a comment that starts with four slashes
after the language's comment opener and says why (`// //// Neoffice — …` in Rust, `# //// Neoffice — …` in TOML and YAML,
`<!-- //// Neoffice — … -->` in Markdown), so that `grep -rn "////"` maps the whole divergence at the next upstream merge.
A file this fork adds says so in its first lines. `.github/workflows/neoffice-markers.yml` checks it on every push.

## Base of the divergence

- `73cbee4` (upstream `main` on 2026-10-20 in the morning: SPDX lines in every Rust file and the `cargo xtask spdx` check,
  one pinned Rust version for CI, the Scorecard and reproducible-build workflows, and the two roadmap documents), merged into
  `version-15`. The fork's own Rust files got their SPDX lines by `cargo xtask spdx --fix` in the commit after the merge.
- Before it: `d7db890` (0.5.2, the desktop alerts, the project's new Microsoft registration, About ▸ Testers, and the
  governance documents), `ed2bd41` (upstream `main` on 2026-10-19 in the evening: 0.5.1, the security review fixes and the fuzz
  targets), and `d33985a` (upstream `main` on 2026-10-19: v0.5.0 and the contributor licence agreement), where the fork
  was created.

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
  its id on the server, never its text, the sample mailbox's by its text, any other mailbox not at all; the origin of
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
- This file.

## What cannot carry a comment

A `Cargo.lock` change or a binary asset that differs from upstream gets a line here, and so does any file of a format
the marker check does not know as one that takes comments:

- `Cargo.lock`: the `mailcraft-jmap` package (with its dependencies on `mailcraft-mime`, `mailcraft-model` and
  `mailcraft-time`), the dependencies on it of the web app and, for its tests, the command line, and nothing else.
- `crates/ui-egui/src/i18n/fr.tsv`: the French interface catalog. Its first line is a `#` comment that says it is ours
  (the format has comments), but the check reads a `.tsv` as text without comment syntax. Each catalog added next to
  it (`crates/ui-egui/src/i18n/<code>.tsv`) is named here the same way.
