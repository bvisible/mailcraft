// //// Neoffice — added file (no upstream equivalent)
//! `http(s)://` addresses: parsed, resolved against a base, and percent-encoding.
//!
//! Addresses reach this crate from the owner (the session's address) and from a server (the
//! session's `apiUrl`, `downloadUrl` and `uploadUrl`), so nothing here trusts them: they are bounded,
//! they may hold no user name (`https://user:password@host/`), no backslash and no control
//! character before the path, and what a parse refuses is never echoed back, since an address can
//! hold a person's address or a password.

use crate::{Error, Result};

/// Longest address parsed.
const MAX_URL: usize = 8192;

/// A parsed `http(s)://host[:port]/path?query` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

pub fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    s.get(..p.len()).filter(|h| h.eq_ignore_ascii_case(p)).and_then(|_| s.get(p.len()..))
}

impl Url {
    pub fn parse(s: &str) -> Result<Url> {
        let s = s.trim();
        let bad = || Error::Protocol("not a web address".into());
        if s.len() > MAX_URL {
            return Err(bad());
        }
        let (https, rest) = if let Some(r) = strip_prefix_ci(s, "https://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(s, "http://") {
            (false, r)
        } else {
            return Err(bad());
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.is_empty() || authority.contains(['@', '\\']) || authority.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| bad())?,
                None if after.is_empty() => default_port(https),
                None => return Err(bad()),
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                // A colon inside the host is no port's: an IPv6 address is written in brackets.
                Some((h, _)) if h.contains(':') => return Err(bad()),
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad())?),
                None => (authority.to_string(), default_port(https)),
            }
        };
        if host.is_empty() || port == 0 {
            return Err(bad());
        }
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        // Spaces and controls are percent-encoded (every byte of their UTF-8) rather than sent raw.
        let mut encoded = String::with_capacity(path.len());
        for c in path.chars() {
            if c == ' ' || c.is_control() {
                let mut utf8 = [0u8; 4];
                for b in c.encode_utf8(&mut utf8).bytes() {
                    encoded.push_str(&format!("%{b:02X}"));
                }
            } else {
                encoded.push(c);
            }
        }
        Ok(Url { https, host: host.to_ascii_lowercase(), port, path: encoded })
    }

    /// A `Location` (or any reference) resolved against this URL.
    pub fn join(&self, location: &str) -> Result<Url> {
        let l = location.trim();
        // A reference with a scheme of its own is absolute (RFC 3986 §4.2): http and https are parsed,
        // any other is refused, rather than taken for a path.
        if let Some((scheme, _)) = l.split_once(':')
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Url::parse(l);
        }
        let origin = self.origin();
        if let Some(rest) = l.strip_prefix("//") {
            return Url::parse(&format!("{}://{rest}", if self.https { "https" } else { "http" }));
        }
        if l.starts_with('/') {
            return Url::parse(&format!("{origin}{l}"));
        }
        let dir = self.path.split('?').next().unwrap_or("/").rsplit_once('/').map_or("", |(d, _)| d);
        Url::parse(&format!("{origin}{dir}/{l}"))
    }

    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == default_port(self.https) { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{}", self.port) }
    }

    /// The whole address again.
    pub fn full(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// Whether the address is on this computer: `localhost`, or an IPv4 address in `127.0.0.0/8`
    /// or `::1` written out in full. The only place plain `http` is allowed to go to.
    pub fn is_loopback(&self) -> bool {
        self.host == "localhost" || self.host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }
}

/// Percent-encode everything but RFC 3986's unreserved characters.
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn urls_and_encodings() {
        let u = Url::parse("https://Api.Example.com/jmap/session?x=1#frag").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("api.example.com", 443, "/jmap/session?x=1"));
        assert_eq!(u.join("/api/").unwrap().full(), "https://api.example.com/api/");
        assert_eq!(u.join("upload/").unwrap().full(), "https://api.example.com/jmap/upload/");
        assert_eq!(u.join("http://other:8080/x").unwrap().full(), "http://other:8080/x");
        for bad in ["ftp://x", "https://", "https://user@host/", "https://h:0/", "https://[::1/"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Url::parse("http://[::1]:8080").unwrap().origin(), "http://[::1]:8080");
        assert_eq!(pct("a b/é"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn references_resolve_against_the_base() {
        let base = Url::parse("https://h.example:8443/a/b/session?account=x").unwrap();
        assert_eq!(base.join("//other.example/p").unwrap().full(), "https://other.example/p");
        assert_eq!(base.join("/p?q=1").unwrap().full(), "https://h.example:8443/p?q=1");
        assert_eq!(base.join("c").unwrap().full(), "https://h.example:8443/a/b/c");
        assert_eq!(base.join(" https://x.example ").unwrap().full(), "https://x.example/");
        // The base's scheme is the reference's when it names none.
        let plain = Url::parse("http://127.0.0.1:8080/s").unwrap();
        assert_eq!(plain.join("//127.0.0.1:9/p").unwrap().full(), "http://127.0.0.1:9/p");
        assert_eq!(Url::parse("HTTPS://H.Example").unwrap().full(), "https://h.example/");
        assert_eq!(Url::parse("https://h.example?a=1").unwrap().path, "/?a=1");
        // Another scheme is no path; a colon further into a path is none of a scheme's.
        for other in ["ftp://h.example/x", "javascript:alert(1)", "mailto:a@b.example", "c:d", "https:x"] {
            assert!(base.join(other).is_err(), "{other}");
        }
        assert_eq!(base.join("a/b:c").unwrap().full(), "https://h.example:8443/a/b/a/b:c");
        assert_eq!(base.join("./c:d").unwrap().full(), "https://h.example:8443/a/b/./c:d");
    }

    #[test]
    fn hostile_addresses_are_refused_without_being_echoed() {
        for bad in [
            "https://user:secret@host.example/",
            "https://host.example\\@evil.example/",
            "https://good.example\\.evil.example/",
            "https://good.example\\path",
            "https://ho st.example/",
            "https://host.example:99999/",
            "https://host.example:/",
            "https://host.example:80:80/",
            "https://::1/",
            "https://[]/",
            "https://[::1]x/",
            "https://\u{7}host/",
            "javascript:alert(1)",
            "",
        ] {
            let err = Url::parse(bad).unwrap_err().to_string();
            assert!(!err.contains("secret") && !err.contains("host"), "{bad}: {err}");
        }
        let long = format!("https://h.example/{}", "a".repeat(MAX_URL));
        assert!(Url::parse(&long).is_err());
        // Controls and spaces of a path are encoded whole, however many bytes their UTF-8 takes.
        let u = Url::parse("https://h.example/a b\r\nc\u{85}d?e=f g").unwrap();
        assert_eq!(u.path, "/a%20b%0D%0Ac%C2%85d?e=f%20g");
        assert!(!u.full().contains(['\r', '\n', ' ']));
    }

    #[test]
    fn only_this_computer_is_loopback() {
        for yes in ["http://localhost/", "http://127.0.0.1:8080/", "http://127.9.9.9/", "http://[::1]/", "https://LOCALHOST/"] {
            assert!(Url::parse(yes).unwrap().is_loopback(), "{yes}");
        }
        for no in [
            "http://127.0.0.1.evil.example/",
            "http://localhost.evil.example/",
            "http://10.0.0.1/",
            "http://128.0.0.1/",
            "http://[::2]/",
            "http://127.1/",
            "http://example.org/",
        ] {
            assert!(!Url::parse(no).unwrap().is_loopback(), "{no}");
        }
    }

    proptest! {
        #[test]
        fn any_address_parses_or_is_refused(s in ".{0,300}", rel in ".{0,100}") {
            if let Ok(u) = Url::parse(&s) {
                prop_assert!(!u.host.is_empty() && u.port != 0 && u.path.starts_with('/'));
                prop_assert!(!u.path.contains([' ', '\r', '\n']));
                // What was parsed parses again to the same address.
                prop_assert_eq!(Url::parse(&u.full()).ok(), Some(u.clone()));
                let _ = u.join(&rel);
                let _ = u.is_loopback();
            }
            let _ = pct(&s);
        }
    }
}
