// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): where Nora's calls from the browser build may go (lifted from the editors' nora_origin.rs).

//! Where Nora's calls may go, and which of them carry the page's credentials. The rules are those of
//! every Neoffice editor (WordCraft's `nora_origin.rs`, which this is, closed maintenance#1661 there):
//! - `?nora` with no value (or `1`, `true`) is the page's own origin (`location.origin`);
//! - `?nora=<origin>` is taken only when it is the page's own origin, compared as the browser compares
//!   origins (the case, a trailing slash and the scheme's default port make no difference), or when the
//!   page and that origin are both loopback (`127.0.0.1`, `localhost`, `[::1]`: the workshop's stand-in
//!   server); any other origin the address names is refused, and the calls go to the page's own origin;
//! - no other origin is taken from anywhere (the old base's `window.mailcraftNora` is gone: no page set
//!   it); a hosting page that makes the calls itself offers `window.mailcraftNoraCall` instead;
//! - the page's credentials (a Frappe page's CSRF token) go only with a call to the page's own origin.
//!
//! So a link to an instance's NeoMail cannot send what the person writes, a message's text or its id, or
//! the person's session token, to a server the link names. The rules have no browser type in them, so
//! they are tested natively; `nora.rs` applies them.

// Outside the browser only the tests apply the rules.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// The hosts of this computer: a page served from one may send Nora's calls to another (the workshop's
/// stand-in server, on its own port).
const LOOPBACK: &[&str] = &["127.0.0.1", "localhost", "[::1]"];

/// Where Nora's calls go when the page's address has `nora=<param>` (`param` as `URLSearchParams` reads
/// it, empty for a bare `?nora`), for a page whose own origin is `page` (`location.origin`): `Ok("")`
/// for the page's own origin (the calls go to paths of the page), `Ok(origin)` for a loopback stand-in.
/// `Err` says why an origin the address names is not taken; the calls then go to the page's own origin.
pub fn from_param(param: &str, page: &str) -> Result<String, String> {
    let param = param.trim();
    if matches!(param, "" | "1" | "true") {
        return Ok(String::new());
    }
    match (Origin::parse(param), Origin::parse(page)) {
        (Some(named), Some(own)) if named == own => Ok(String::new()),
        (Some(named), Some(own)) if named.is_loopback() && own.is_loopback() => Ok(named.to_string()),
        _ => Err(format!(
            "?nora={param} is not taken: a page's address may name only the page's own origin for Nora (or, from a loopback \
             page, a loopback one), so her calls go to the page's own origin; a hosting page that makes them itself offers \
             window.mailcraftNoraCall"
        )),
    }
}

/// Whether a call to `target` (an origin, or `""` for a path of the page's own) may carry the page's
/// credentials: only a call to the page's own origin, `page`.
pub fn is_page_origin(target: &str, page: &str) -> bool {
    target.is_empty() || Origin::parse(target).zip(Origin::parse(page)).is_some_and(|(target, page)| target == page)
}

/// An `http` or `https` origin, as the browser compares origins: the scheme and the host lower-cased,
/// the port without the scheme's default.
#[derive(Debug, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    /// `scheme://host[:port]`, with one `/` after it at most; `None` for anything else: a user, a path,
    /// a query, a fragment, an escape, another scheme, an opaque origin (`null`).
    fn parse(text: &str) -> Option<Origin> {
        let text = text.trim();
        let text = text.strip_suffix('/').unwrap_or(text);
        let (scheme, rest) = text.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => return None,
        };
        let (host, port) = match rest.strip_prefix('[') {
            // An IPv6 address, `[::1]:8080`.
            Some(inside) => {
                let (address, tail) = inside.split_once(']')?;
                if address.is_empty() || !address.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
                    return None;
                }
                (format!("[{}]", address.to_ascii_lowercase()), if tail.is_empty() { "" } else { tail.strip_prefix(':')? })
            }
            None => {
                let (host, port) = rest.split_once(':').unwrap_or((rest, ""));
                if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
                    return None;
                }
                (host.to_ascii_lowercase(), port)
            }
        };
        let port = if port.is_empty() { None } else { Some(port.parse::<u16>().ok()?) };
        Some(Origin { scheme, host, port: port.filter(|port| *port != default_port) })
    }

    fn is_loopback(&self) -> bool {
        LOOPBACK.contains(&self.host.as_str())
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.port {
            Some(port) => write!(f, "{}://{}:{port}", self.scheme, self.host),
            None => write!(f, "{}://{}", self.scheme, self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://drive.example";
    const LOOPBACK_PAGE: &str = "http://127.0.0.1:8791";

    #[test]
    fn a_bare_nora_is_the_page_s_own_origin() {
        for param in ["", "1", "true"] {
            assert_eq!(from_param(param, PAGE), Ok(String::new()), "{param:?}");
        }
        assert_eq!(from_param("", "null"), Ok(String::new()), "a page without an origin of its own calls its own paths too");
    }

    #[test]
    fn the_page_s_own_origin_may_be_named() {
        for param in
            ["https://drive.example", "https://drive.example/", "HTTPS://Drive.Example", "https://drive.example:443", " https://drive.example "]
        {
            assert_eq!(from_param(param, PAGE), Ok(String::new()), "{param:?}");
        }
        assert_eq!(from_param("http://localhost:8080/", "http://LOCALHOST:8080"), Ok(String::new()));
    }

    #[test]
    fn any_other_origin_named_by_the_address_is_refused() {
        for param in [
            "https://evil.example",
            "http://drive.example",
            "https://drive.example:8443",
            "https://drive.example.evil.example",
            "https://drive.example@evil.example",
            "https://user:pw@drive.example",
            "https://evil.example/https://drive.example",
            "https://drive.example/api",
            "https://drive.example?x=1",
            "https://drive.example#x",
            "https://drive.example\\@evil.example",
            "https://drive%2eexample",
            "//evil.example",
            "javascript:alert(1)",
            "data:text/html,x",
            "evil.example",
            "drive.example",
            "https://",
            "https://:443",
            "https://drive.example:99999",
            "http://127.0.0.1:8773",
        ] {
            let refused = from_param(param, PAGE);
            assert!(refused.as_ref().is_err_and(|why| why.contains("the page's own origin")), "{param:?} was taken: {refused:?}");
        }
        assert!(from_param("https://drive.example", "null").is_err(), "a page without an origin of its own has none to match");
    }

    #[test]
    fn a_loopback_page_may_name_a_loopback_origin_for_the_workshop_s_mock() {
        assert_eq!(from_param("http://127.0.0.1:8773", LOOPBACK_PAGE), Ok("http://127.0.0.1:8773".into()));
        assert_eq!(from_param("http://LOCALHOST:8773/", LOOPBACK_PAGE), Ok("http://localhost:8773".into()));
        assert_eq!(from_param("http://[::1]:8773", "http://localhost:8080"), Ok("http://[::1]:8773".into()));
        assert_eq!(from_param("http://127.0.0.1:80", "http://[::1]:8080"), Ok("http://127.0.0.1".into()), "without the scheme's default port");
        for param in [
            "https://evil.example",
            "http://127.0.0.1.evil.example:8773",
            "http://127.0.0.2:8773",
            "http://localhost@evil.example",
            "http://localhost.:8773",
        ] {
            assert!(from_param(param, LOOPBACK_PAGE).is_err(), "{param:?}");
        }
        assert!(from_param("http://127.0.0.1:8773", PAGE).is_err(), "not from the page of an instance");
    }

    #[test]
    fn the_page_s_credentials_go_only_to_its_own_origin() {
        assert!(is_page_origin("", PAGE), "a path of the page's own origin");
        assert!(is_page_origin("https://drive.example", PAGE));
        assert!(is_page_origin("https://drive.example/", PAGE));
        assert!(is_page_origin("HTTPS://DRIVE.example:443", PAGE));
        assert!(!is_page_origin("https://evil.example", PAGE));
        assert!(!is_page_origin("https://nora.drive.example", PAGE), "another host, even one under the page's");
        assert!(!is_page_origin("http://drive.example", PAGE), "another scheme is another origin");
        assert!(!is_page_origin("http://127.0.0.1:8773", LOOPBACK_PAGE), "another port is another origin, loopback or not");
        assert!(!is_page_origin("not an origin", PAGE));
        assert!(!is_page_origin("https://drive.example", "null"), "a page without an origin of its own has none to match");
    }
}
