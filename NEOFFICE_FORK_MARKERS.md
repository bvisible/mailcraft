<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

This is Neoffice's fork of [UCS-Care-Group/mailcraft](https://github.com/UCS-Care-Group/mailcraft). Its only branch,
`version-15`, is what Neoffice publishes. Every change to upstream's files carries a comment that starts with four slashes
after the language's comment opener and says why (`// //// Neoffice — …` in Rust, `# //// Neoffice — …` in TOML and YAML,
`<!-- //// Neoffice — … -->` in Markdown), so that `grep -rn "////"` maps the whole divergence at the next upstream merge.
A file this fork adds says so in its first lines. `.github/workflows/neoffice-markers.yml` checks it on every push.

## Base of the divergence

- `d33985a` (upstream `main` on 2026-10-19: v0.5.0 and the contributor licence agreement), where the fork was created.

## What the fork adds

- `.github/workflows/neoffice-markers.yml`: the marker check above.
- `.github/workflows/neoffice-ci.yml`: upstream's gate (`cargo xtask ci`, then `cargo xtask deny`) in one job on a GitHub
  runner, and a `cargo check` of the web app for wasm32, which `cargo xtask wasm` leaves out; upstream's own workflows are
  disabled on this fork.
- `.github/workflows/neoffice-web.yml`: the browser build by upstream's packaging script, kept as an artifact or published
  as a release (`web-<version>-<short sha>`).
- `crates/jmap/` (`mailcraft-jmap`, layer 3): JMAP (RFC 8620 and 8621) without sockets or browser APIs, so that mail can be
  read over HTTPS in the browser, where IMAP and SMTP can't go. An async client over an `Http` trait, the address helpers,
  `sync_mail` (an account's mailboxes and the newest messages of each, for the engine), the raw octets of a
  message, fetched by its email id (`fetch`), and an in-process server for the tests of the crates that sit on it
  (the `stand-in` feature). A crate with no upstream equivalent, written so that it can be offered upstream: nothing
  in it names this fork's products.
- `apps/mailcraft-cli/tests/jmap_list.rs` and `jmap_fetch.rs`: the engine, `sync_mail` and `fetch` against that
  in-process server, end to end.
- This file.

## What cannot carry a comment

A `Cargo.lock` change or a binary asset that differs from upstream gets a line here, and so does any file of a format
the marker check does not know as one that takes comments:

- `Cargo.lock`: the `mailcraft-jmap` package (with its dependencies on `mailcraft-mime`, `mailcraft-model` and
  `mailcraft-time`), the dependencies on it of the web app and, for its tests, the command line, and nothing else.
- `crates/ui-egui/src/i18n/fr.tsv`: the French interface catalog. Its first line is a `#` comment that says it is ours
  (the format has comments), but the check reads a `.tsv` as text without comment syntax. Each catalog added next to
  it (`crates/ui-egui/src/i18n/<code>.tsv`) is named here the same way.
