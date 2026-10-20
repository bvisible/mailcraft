// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The stand-in on this computer's loopback, for a browser to talk to: a window built for the browser, in a hosting
//! page served from this computer, reads the demo fortnight ([`super::demo`]) from it as it would read a server's.
//!
//! Only in this crate's own test build, and only when asked for by hand:
//!
//! ```text
//! MAILCRAFT_STANDIN_PORT=8790 MAILCRAFT_STANDIN_TODAY=2026-10-20 \
//!   cargo test -p mailcraft-jmap --features stand-in -- --ignored serve_the_stand_in_on_loopback --nocapture
//! ```
//!
//! It listens on `127.0.0.1` alone (the port from `MAILCRAFT_STANDIN_PORT`, 8790 when unset; the demo around the day
//! `MAILCRAFT_STANDIN_TODAY` names, else today, with as many more events as `MAILCRAFT_STANDIN_EVENTS` says and as many
//! more cards as `MAILCRAFT_STANDIN_CARDS` says, for a look at what they cost the tab), takes one connection at a time
//! on one thread, and answers each request once (`Connection: close`):
//!
//! - **Every answer carries CORS headers** that let any page call it with a credential in a header (the browser's
//!   preflight, an `OPTIONS`, is answered at once with 204): it is a test server on this computer, which a page of
//!   another port of this computer must reach. `Retry-After` is exposed to the page.
//! - **Trouble on demand**, by a `GET` of `/__trouble` with a query: `?down` (from then on every request is closed
//!   without an answer, as a server that is gone), `?up` (well again, and quick), `?slow=<ms>` (each answer waits that
//!   long first, a minute at most), `?expire` (the token it holds now is no longer good: it holds another, which
//!   `GET /__token` gives, as a hosting page's renewal would). `GET /__log` lists the methods it was asked to call,
//!   `GET /__written` the changes of events it was sent, and `GET /__mail` the mail it would have sent about them.
//! - Anything else is the stand-in's, at its own address: the session (`/jmap/session`, whose other addresses are
//!   relative to it), the API, the downloads.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use super::{ORIGIN, StandIn, TOKEN};
use crate::{Http as _, Request, block_on};

/// The largest head and body of a request it reads.
const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 16 << 20;
/// The longest an answer is made to wait.
const MAX_SLOW_MS: u64 = 60_000;

/// What has been asked of it in the way of trouble.
#[derive(Debug, Default)]
pub(super) struct Trouble {
    pub down: bool,
    pub slow_ms: u64,
    /// How many times its token was made to expire.
    pub expired: u32,
}

/// The headers every answer carries.
const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Accept\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Expose-Headers: Retry-After\r\nAccess-Control-Max-Age: 600\r\n";

/// An answer, written out.
fn reply(status: u16, kind: &str, body: &[u8], retry_after: Option<u32>) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let wait = retry_after.map(|seconds| format!("Retry-After: {seconds}\r\n")).unwrap_or_default();
    let mut out =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{CORS}{wait}Connection: close\r\n\r\n", body.len())
            .into_bytes();
    out.extend_from_slice(body);
    out
}

/// The answer to one request as it came over the wire (`raw`, head and body), or `None` to close the connection
/// without one.
pub(super) fn answer(stand_in: &StandIn, trouble: &mut Trouble, raw: &[u8]) -> Option<Vec<u8>> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(raw.get(..end)?).to_string();
    let body = raw.get(end + 4..).unwrap_or_default().to_vec();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> =
        lines.filter_map(|line| line.split_once(':')).map(|(name, value)| (name.trim().to_string(), value.trim().to_string())).collect();
    if method == "OPTIONS" {
        return Some(reply(204, "text/plain", b"", None));
    }
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    match path {
        "/__trouble" => {
            for word in query.split('&') {
                match word.split_once('=').unwrap_or((word, "")) {
                    ("down", _) => trouble.down = true,
                    ("up", _) => (trouble.down, trouble.slow_ms) = (false, 0),
                    ("slow", ms) => trouble.slow_ms = ms.parse::<u64>().unwrap_or(0).min(MAX_SLOW_MS),
                    ("expire", _) => {
                        trouble.expired = trouble.expired.saturating_add(1);
                        let next = format!("{TOKEN}-{}", trouble.expired);
                        stand_in.with(|d| d.token = Some(next));
                    }
                    _ => {}
                }
            }
            return Some(reply(200, "application/json", format!("{trouble:?}").as_bytes(), None));
        }
        "/__token" => {
            let token = stand_in.with(|d| d.token.clone()).unwrap_or_default();
            return Some(reply(200, "text/plain", token.as_bytes(), None));
        }
        "/__log" => {
            let calls = serde_json::to_vec(&stand_in.calls()).unwrap_or_default();
            return Some(reply(200, "application/json", &calls, None));
        }
        "/__mail" => {
            let sent: Vec<serde_json::Value> = stand_in.with(|d| {
                d.pim
                    .scheduled
                    .iter()
                    .map(|m| serde_json::json!({"what": m.what, "event": m.event, "occurrence": m.occurrence, "to": m.to}))
                    .collect()
            });
            return Some(reply(200, "application/json", &serde_json::to_vec(&sent).unwrap_or_default(), None));
        }
        "/__written" => {
            let written: Vec<serde_json::Value> =
                stand_in.with(|d| d.pim.written.iter().map(|(id, patch)| serde_json::json!({"id": id, "patch": patch})).collect());
            return Some(reply(200, "application/json", &serde_json::to_vec(&written).unwrap_or_default(), None));
        }
        _ => {}
    }
    if trouble.down {
        return None;
    }
    if trouble.slow_ms > 0 {
        std::thread::sleep(Duration::from_millis(trouble.slow_ms));
    }
    let request = Request { method, url: format!("{ORIGIN}{target}"), headers, body, max: crate::MAX_JSON.max(MAX_BODY) };
    let kind = if path.starts_with("/jmap/download/") { "application/octet-stream" } else { "application/json" };
    // An answer the stand-in loses (`lost_answers`) is a connection closed without one.
    let resp = block_on(stand_in.send(request)).ok()?;
    Some(reply(resp.status, kind, &resp.body, resp.retry_after))
}

/// One connection: its request read whole (within bounds and a few seconds), answered, closed.
pub(super) fn serve_one(mut stream: TcpStream, stand_in: &StandIn, trouble: &mut Trouble) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut wanted: Option<usize> = None;
    loop {
        if let Some(total) = wanted
            && raw.len() >= total
        {
            break;
        }
        let Ok(read) = stream.read(&mut chunk) else { return };
        if read == 0 {
            break;
        }
        raw.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if raw.len() > MAX_HEAD + MAX_BODY {
            return;
        }
        if wanted.is_none()
            && let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n")
        {
            let head = String::from_utf8_lossy(raw.get(..end).unwrap_or_default()).to_ascii_lowercase();
            let length = head.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0);
            wanted = Some(end + 4 + length.min(MAX_BODY));
        }
    }
    if let Some(answer) = answer(stand_in, trouble, &raw) {
        let _ = stream.write_all(&answer);
    }
    let _ = stream.flush();
}

/// Serve `stand_in` on `127.0.0.1:port` until the process is stopped.
pub(super) fn serve(stand_in: &StandIn, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let mut trouble = Trouble::default();
    for stream in listener.incoming().flatten() {
        serve_one(stream, stand_in, &mut trouble);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
