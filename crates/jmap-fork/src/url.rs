// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A parsed `http` or `https` address, as a JMAP client (RFC 8620) needs one: the session resource, the API,
//! upload and download endpoints, and redirects.
//!
//! Syntax and reference resolution follow RFC 3986; host parsing, default ports and the origin follow the
//! WHATWG URL Standard; [`pct`] is RFC 6570 simple string expansion. Every error is
//! [`Error::Protocol`] with a short reason that never repeats the address (an address may carry a token).

use std::fmt;

use crate::http::Error;

/// Longest address accepted, in bytes.
const MAX_ADDRESS: usize = 8192;
/// Longest host accepted, in bytes.
const MAX_HOST: usize = 253;

/// The default port of the scheme: 443 for `https`, 80 for `http`.
pub fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

/// A parsed `http` or `https` address.
///
/// Built by [`Url::parse`] or [`Url::join`], which guarantee the field invariants below.
#[derive(Clone, PartialEq, Eq)]
pub struct Url {
    /// `true` for `https`, `false` for `http`.
    pub https: bool,
    /// The host, lower-case. An IPv6 literal keeps its brackets, e.g. `"[::1]"`.
    pub host: String,
    /// The explicit port, or [`default_port`] of the scheme when the address gives none.
    pub port: u16,
    /// The path, starting with `/`, followed by `?query` when there is one. Never the `#fragment`.
    pub path: String,
}

impl Url {
    /// Parse an absolute `http://` or `https://` address.
    ///
    /// Refused (with [`Error::Protocol`]): surrounding or inner whitespace, control characters, backslashes,
    /// more than 8192 bytes, another scheme, user information (`@`), an empty or malformed host, a host over
    /// 253 bytes, a port outside 1..=65535. The fragment is dropped; dot segments of the path are removed;
    /// an empty path becomes `/`.
    pub fn parse(s: &str) -> Result<Url, Error> {
        check_text(s)?;
        let (https, rest) = split_scheme(s).ok_or_else(|| invalid("the address does not start with http:// or https://"))?;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at_checked(authority_end).ok_or_else(|| invalid("the address cannot be split"))?;
        if authority.contains('@') {
            return Err(invalid("the address holds user information"));
        }
        let (host, port) = split_host_port(authority)?;
        let host = parse_host(host)?;
        let port = parse_port(port, https)?;
        let without_fragment = tail.split_once('#').map_or(tail, |(before, _)| before);
        Ok(Url { https, host, port, path: normalize_path(without_fragment) })
    }

    /// Resolve `location` (a redirect `Location` or a link a server sent) against this address, following
    /// RFC 3986 §5.2, and validate the result as [`Url::parse`] would.
    ///
    /// An absolute `http:`/`https:` address is parsed as is; any other scheme is refused; `//host…` keeps the
    /// scheme; `/path…` keeps scheme, host and port; `?query` keeps the path and replaces the query; an empty
    /// reference gives this address back; anything else is taken relative to the directory of this path, and
    /// dot segments are removed. The fragment of `location` is always dropped.
    pub fn join(&self, location: &str) -> Result<Url, Error> {
        check_text(location)?;
        let reference = location.split_once('#').map_or(location, |(before, _)| before);
        if let Some(scheme) = scheme_of(reference) {
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
                return Url::parse(reference);
            }
            return Err(invalid("the link leads to a scheme other than http or https"));
        }
        if reference.starts_with("//") {
            let scheme = if self.https { "https:" } else { "http:" };
            return Url::parse(&format!("{scheme}{reference}"));
        }
        if reference.starts_with('/') {
            return Url::parse(&format!("{}{reference}", self.origin()));
        }
        // A `Url` built by hand may break the "path starts with /" invariant; never let its path run into
        // the host.
        let path = self.path.as_str();
        let slash = if path.starts_with('/') { "" } else { "/" };
        if reference.is_empty() {
            return Url::parse(&format!("{}{slash}{path}", self.origin()));
        }
        let base_path = path.split_once('?').map_or(path, |(before, _)| before);
        if reference.starts_with('?') {
            return Url::parse(&format!("{}{slash}{base_path}{reference}", self.origin()));
        }
        let directory = base_path.rfind('/').and_then(|last| base_path.get(..=last)).filter(|directory| directory.starts_with('/')).unwrap_or("/");
        Url::parse(&format!("{}{directory}{reference}", self.origin()))
    }

    /// `"https://host"` or `"http://host"`, with `":port"` only when the port is not the scheme's default.
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        if self.port == default_port(self.https) { format!("{scheme}://{}", self.host) } else { format!("{scheme}://{}:{}", self.host, self.port) }
    }

    /// The whole address: [`Url::origin`] followed by the path (and query). Parses back to an equal `Url`.
    pub fn full(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// Whether the host is the local machine: `localhost` or a name ending in `.localhost`, an IPv4 address
    /// in 127.0.0.0/8 written as four decimal numbers, or the IPv6 address `::1`.
    pub fn is_loopback(&self) -> bool {
        let host = self.host.as_str();
        if host == "localhost" || host.ends_with(".localhost") {
            return true;
        }
        if let Some(inside) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            return parse_ipv6(inside) == Some([0, 0, 0, 0, 0, 0, 0, 1]);
        }
        matches!(parse_ipv4(host), Some([127, _, _, _]))
    }
}

impl fmt::Debug for Url {
    /// Writes [`Url::origin`] only: the path and query can carry a token.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.origin())
    }
}

/// Percent-encode the UTF-8 bytes of `s`, keeping only the RFC 3986 unreserved characters
/// `A-Z a-z 0-9 - . _ ~`; every other byte becomes `%XX` with upper-case hex.
///
/// This is RFC 6570 simple string expansion, used to fill `{accountId}`, `{blobId}`, `{name}` and `{type}` in
/// JMAP's URL templates (RFC 8620 §2).
pub fn pct(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            for nibble in [byte >> 4, byte & 0x0f] {
                let digit = HEX.get(usize::from(nibble)).copied().unwrap_or(b'0');
                out.push(char::from(digit));
            }
        }
    }
    out
}

/// Replace each `%XX` (two hex digits, either case) with its byte, leave any other `%` as it is, and turn the
/// bytes into a string, replacing invalid UTF-8 with U+FFFD. `+` stays `+`.
pub fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'%' {
            let high = bytes.get(i + 1).copied().and_then(hex_value);
            let low = bytes.get(i + 2).copied().and_then(hex_value);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high << 4) | low);
                i += 3;
                continue;
            }
        }
        out.push(byte);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of one hex digit, either case.
fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).and_then(|digit| u8::try_from(digit).ok())
}

/// A protocol error with a fixed reason.
fn invalid(reason: &str) -> Error {
    Error::Protocol(reason.to_owned())
}

/// The checks that apply to the whole text of an address or a reference: size and characters.
fn check_text(s: &str) -> Result<(), Error> {
    if s.len() > MAX_ADDRESS {
        return Err(invalid("the address is too long"));
    }
    if s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid("the address holds a space or a control character"));
    }
    if s.contains('\\') {
        return Err(invalid("the address holds a backslash"));
    }
    Ok(())
}

/// `Some((https, rest))` when `s` starts with `https://` or `http://` (any case).
fn split_scheme(s: &str) -> Option<(bool, &str)> {
    if let Some(rest) = strip_prefix_ignore_case(s, "https://") {
        return Some((true, rest));
    }
    strip_prefix_ignore_case(s, "http://").map(|rest| (false, rest))
}

/// `s` without `prefix`, matched ASCII case-insensitively.
fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) { s.get(prefix.len()..) } else { None }
}

/// The scheme of a URI reference (RFC 3986 §3.1, §4.2), when it has one.
fn scheme_of(reference: &str) -> Option<&str> {
    let (candidate, _) = reference.split_once(':')?;
    let mut chars = candidate.chars();
    let first = chars.next()?;
    let valid = first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(candidate)
}

/// Split an authority (already free of user information) into host and optional port text.
fn split_host_port(authority: &str) -> Result<(&str, Option<&str>), Error> {
    if authority.starts_with('[') {
        let close = authority.find(']').ok_or_else(|| invalid("the IPv6 address is not closed"))?;
        let (host, after) = authority.split_at_checked(close + 1).ok_or_else(|| invalid("the address cannot be split"))?;
        if after.is_empty() {
            return Ok((host, None));
        }
        return match after.strip_prefix(':') {
            Some(port) => Ok((host, Some(port))),
            None => Err(invalid("unexpected characters after the host")),
        };
    }
    Ok(match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    })
}

/// Validate a host and return it lower-cased.
fn parse_host(host: &str) -> Result<String, Error> {
    if host.is_empty() {
        return Err(invalid("the address has no host"));
    }
    if host.len() > MAX_HOST {
        return Err(invalid("the host is too long"));
    }
    if let Some(inside) = host.strip_prefix('[') {
        let inside = inside.strip_suffix(']').ok_or_else(|| invalid("the IPv6 address is not closed"))?;
        if !inside.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return Err(invalid("the IPv6 address holds characters it cannot"));
        }
        if parse_ipv6(inside).is_none() {
            return Err(invalid("the IPv6 address is not valid"));
        }
        return Ok(host.to_ascii_lowercase());
    }
    if !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Err(invalid("the host holds characters a host cannot"));
    }
    // A host whose last label is a number is an IPv4 address to a browser, which reads forms such as
    // `127.1`, `0x7f.0.0.1` or `2130706433` as 127.0.0.1. Only the four-decimal form means the same thing
    // to everyone, so it is the only one accepted.
    if ends_in_number(host) && parse_ipv4(host).is_none() {
        return Err(invalid("the IPv4 address is not four decimal numbers"));
    }
    Ok(host.to_ascii_lowercase())
}

/// The port from its text: absent or empty means the scheme's default; otherwise 1 to 5 digits, 1..=65535.
fn parse_port(port: Option<&str>, https: bool) -> Result<u16, Error> {
    let text = match port {
        None | Some("") => return Ok(default_port(https)),
        Some(text) => text,
    };
    if text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("the port is not valid"));
    }
    match text.parse::<u16>() {
        Ok(value) if value != 0 => Ok(value),
        _ => Err(invalid("the port is not valid")),
    }
}

/// The path from the text after the authority (fragment already dropped): `/` in front when missing, and dot
/// segments removed from the path (not from the query).
fn normalize_path(text: &str) -> String {
    let (path, query) = match text.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (text, None),
    };
    let mut out = remove_dot_segments(path);
    if let Some(query) = query {
        out.push('?');
        out.push_str(query);
    }
    out
}

/// RFC 3986 §5.2.4 for a path that is absolute or empty (an empty path gives `/`). A segment counts as a dot
/// segment in its percent-encoded spellings too (`%2e`, `.%2E`…), as the WHATWG URL Standard reads them.
fn remove_dot_segments(path: &str) -> String {
    let relative = path.strip_prefix('/').unwrap_or(path);
    let segments: Vec<&str> = relative.split('/').collect();
    let last = segments.len().saturating_sub(1);
    let mut kept: Vec<&str> = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let is_last = index == last;
        if is_double_dot(segment) {
            kept.pop();
            if is_last {
                kept.push("");
            }
        } else if is_single_dot(segment) {
            if is_last {
                kept.push("");
            }
        } else {
            kept.push(segment);
        }
    }
    format!("/{}", kept.join("/"))
}

/// `.` or `%2e` (any case).
fn is_single_dot(segment: &str) -> bool {
    segment == "." || segment.eq_ignore_ascii_case("%2e")
}

/// `..`, `.%2e`, `%2e.` or `%2e%2e` (any case).
fn is_double_dot(segment: &str) -> bool {
    segment == ".." || [".%2e", "%2e.", "%2e%2e"].iter().any(|form| segment.eq_ignore_ascii_case(form))
}

/// Whether the last label of `host` is a number (the WHATWG URL Standard's "ends in a number checker"):
/// only ASCII digits, or `0x` followed by hex digits.
fn ends_in_number(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let last = host.rsplit('.').next().unwrap_or(host);
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    let hex = last.strip_prefix("0x").or_else(|| last.strip_prefix("0X"));
    hex.is_some_and(|digits| digits.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// An IPv4 address written as exactly four decimal numbers 0..=255 without leading zeros.
fn parse_ipv4(text: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = text.split('.');
    for octet in &mut octets {
        *octet = parse_decimal_octet(parts.next()?)?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(octets)
}

/// One decimal number 0..=255, without a leading zero (RFC 3986 `dec-octet`).
fn parse_decimal_octet(text: &str) -> Option<u8> {
    let canonical = !text.is_empty() && text.len() <= 3 && text.bytes().all(|b| b.is_ascii_digit()) && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

/// The eight 16-bit pieces of an IPv6 address written as RFC 3986 `IPv6address` (`::` compression and a
/// trailing dotted IPv4 part allowed; no zone).
fn parse_ipv6(text: &str) -> Option<[u16; 8]> {
    let (head, tail, compressed) = match text.split_once("::") {
        Some((head, tail)) => (head, tail, true),
        None => (text, "", false),
    };
    if tail.contains("::") {
        return None;
    }
    // A dotted IPv4 part may only end the whole address.
    let head_pieces = ipv6_pieces(head, !compressed)?;
    let tail_pieces = ipv6_pieces(tail, true)?;
    let given = head_pieces.len() + tail_pieces.len();
    if compressed {
        if given > 7 {
            return None;
        }
    } else if given != 8 {
        return None;
    }
    let zeros = std::iter::repeat_n(0u16, 8 - given);
    let mut pieces = [0u16; 8];
    for (slot, value) in pieces.iter_mut().zip(head_pieces.into_iter().chain(zeros).chain(tail_pieces)) {
        *slot = value;
    }
    Some(pieces)
}

/// The pieces of one side of an IPv6 address (`h16` groups separated by `:`, the last one possibly a dotted
/// IPv4 address worth two pieces). An empty side has no pieces.
fn ipv6_pieces(side: &str, ipv4_may_end: bool) -> Option<Vec<u16>> {
    let mut pieces = Vec::new();
    if side.is_empty() {
        return Some(pieces);
    }
    let mut groups = side.split(':').peekable();
    while let Some(group) = groups.next() {
        let is_last = groups.peek().is_none();
        if group.contains('.') {
            if !(is_last && ipv4_may_end) {
                return None;
            }
            let [a, b, c, d] = parse_ipv4(group)?;
            pieces.push(u16::from_be_bytes([a, b]));
            pieces.push(u16::from_be_bytes([c, d]));
        } else {
            if group.is_empty() || group.len() > 4 {
                return None;
            }
            pieces.push(u16::from_str_radix(group, 16).ok()?);
        }
    }
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_forms() {
        assert_eq!(parse_ipv6("::"), Some([0; 8]));
        assert_eq!(parse_ipv6("::1"), Some([0, 0, 0, 0, 0, 0, 0, 1]));
        assert_eq!(parse_ipv6("1::"), Some([1, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(parse_ipv6("1:2:3:4:5:6:7:8"), Some([1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(parse_ipv6("1:2:3:4:5:6:7::"), Some([1, 2, 3, 4, 5, 6, 7, 0]));
        assert_eq!(parse_ipv6("::2:3:4:5:6:7:8"), Some([0, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(parse_ipv6("::ffff:192.0.2.1"), Some([0, 0, 0, 0, 0, 0xffff, 0xc000, 0x0201]));
        assert_eq!(parse_ipv6("1:2:3:4:5:6:1.2.3.4"), Some([1, 2, 3, 4, 5, 6, 0x0102, 0x0304]));
        assert_eq!(parse_ipv6("2001:DB8::abcd"), Some([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0xabcd]));
        for bad in [
            "",
            ":",
            ":::",
            "1",
            "1:2",
            "1:2:3:4:5:6:7",
            "1:2:3:4:5:6:7:8:9",
            "1:2:3:4:5:6:7:8::",
            "::1:2:3:4:5:6:7:8",
            "1::2::3",
            ":1::",
            "1::2:",
            "12345::",
            "1.2.3.4",
            "1.2.3.4::",
            "::1.2.3",
            "::1.2.3.256",
            "::01.2.3.4",
            "1:2:3:4:5:6:7:1.2.3.4",
        ] {
            assert_eq!(parse_ipv6(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn dot_segments() {
        for (input, expected) in [
            ("", "/"),
            ("/", "/"),
            ("/.", "/"),
            ("/..", "/"),
            ("/a/b/c/./../../g", "/a/g"),
            ("/a/b/c/..", "/a/b/"),
            ("/a/b/c/.", "/a/b/c/"),
            ("/a/./b", "/a/b"),
            ("/a//b/../c", "/a//c"),
            ("/../../g", "/g"),
            ("/a/%2e%2E/b", "/b"),
            ("/a/.%2e/b", "/b"),
            ("/a/%2e./b", "/b"),
            ("/a/%2E/b", "/a/b"),
            ("/a/.../b", "/a/.../b"),
            ("/a/.b/..c", "/a/.b/..c"),
        ] {
            assert_eq!(remove_dot_segments(input), expected, "{input:?}");
        }
    }

    #[test]
    fn number_endings() {
        assert!(ends_in_number("1.2.3.4"));
        assert!(ends_in_number("example.123"));
        assert!(ends_in_number("example.0x1f"));
        assert!(ends_in_number("example.0x"));
        assert!(ends_in_number("1.2.3.4."));
        assert!(!ends_in_number("example.com"));
        assert!(!ends_in_number("example.0xg"));
        assert!(!ends_in_number("123.example"));
        assert!(!ends_in_number("."));
    }
}
