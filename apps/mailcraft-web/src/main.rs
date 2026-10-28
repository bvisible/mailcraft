// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft in the browser.
//!
//! Runs the same [`mailcraft_ui_egui::MailApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`).
//!
//! Differences from the desktop app: no mail-server connections (a browser can't open sockets) and
//! no TCP control channel; the mailbox lives in the browser's storage; Open, Import and Attach use
//! the browser's file picker (bytes arrive through `Services::inbox`); Save and the exports
//! download.
// //// Neoffice — but a JMAP account (RFC 8620, 8621) is read over HTTPS, through `fetch`.
//! An account kept in step with a JMAP server is read over HTTPS, through the browser's `fetch`
//! (`jmap_services.rs`).
//! URL flags: `?webgl` forces WebGL2; `?sample` starts from the sample mailbox; `?empty` from an
//! empty one; neither of those ever writes the mailbox kept in this browser. Without a flag the
//! mailbox kept in this browser opens, or the first time the sample (an empty mailbox when the page
//! names JMAP accounts of its own).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// //// Neoffice — the mailboxes a hosting page hands over (host.rs), and what its answer to a renewal of their tokens
// means for each (renewal.rs, built everywhere so that it is tested natively).
#[cfg(target_arch = "wasm32")]
mod host;
// //// Neoffice — the page that frames a hosting page's window is told whether closing it would lose work (built
// everywhere, so that it is tested natively).
mod host_dirty;
// //// Neoffice — a hosting page that draws the navigation in its own menu is told where the window is and drives it
// (built everywhere, so that it is tested natively).
mod host_menu;
mod renewal;
// //// Neoffice — HTTP through the browser's `fetch`, for JMAP; the runner of a JMAP account's Send/Receive and the
// credentials it is given (built everywhere, so that its tests run natively).
mod credentials;
#[cfg(target_arch = "wasm32")]
#[path = "browser_fetch.rs"]
mod fetch;
// //// Neoffice — the browser-free half of fetch.rs (what a request is built from, how a failure reads, the body's
// size): compiled for the web build, and natively so that its tests run.
#[cfg(any(test, target_arch = "wasm32"))]
mod fetch_rules;
// //// Neoffice — what makes a tab a hosting page's (its runners, and nothing kept between visits), built everywhere so
// that it is tested natively.
mod hosted;
mod jmap_services;
// //// Neoffice — how the browser build reaches Nora, the assistant of the Neoffice server, and where her calls may go
// (both built everywhere, so that their rules are tested natively).
mod nora;
mod nora_origin;
// //// Neoffice — the signature a hosting page gives a mailbox, its logo by its bytes (built everywhere, so that it is
// tested natively).
mod page_signature;
// //// Neoffice — whether the browser's WebGPU gives an adapter in time, asked before the window starts (web.rs).
#[cfg(target_arch = "wasm32")]
mod gpu_probe;
// //// Neoffice — upstream's JMAP host (`jmap.rs`, `jmap_rules.rs` and its `fetch.rs`) reads mail only; this fork's
// //// Neoffice — (jmap_services.rs, host.rs, fetch.rs above) also sends, tells the server of changes and keeps
// //// Neoffice — calendars, contacts and notes, so it is the one built.
mod page;
#[cfg(test)]
mod page_tests;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("mailcraft-web only runs in the browser: build it with `trunk build --release` in apps/mailcraft-web");
}
