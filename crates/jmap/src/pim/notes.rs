// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A person's notes over JMAP. JMAP has no standard for notes: this is a data type of the hosting page's server in its
//! shape (`Note/get`, `Note/query`, `Note/set`; `Note/changes` not kept), under a capability of its own, [`NOTES`].
//! The capability names that server, the one place in this crate that does: it is the name the server gives it, as a
//! vendor's capability is named by its vendor (RFC 8620 §2).

/// The capability of a person's notes, as the hosting page's server names it.
pub const NOTES: &str = "https://neoffice.me/jmap/notes";
