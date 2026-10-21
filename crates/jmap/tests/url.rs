// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Native tests of the `url` module, rule by rule (see SPEC.md and NOTES.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mailcraft_jmap::http::Error;
use mailcraft_jmap::url::{Url, default_port, pct, pct_decode};

/// A secret-looking marker put in addresses, which no error or Debug output may repeat.
const SECRET: &str = "SeCrEt-ToKeN";

fn parse(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("{s:?} was refused: {e}"))
}

/// The reason `s` is refused; also checks that the reason repeats neither the address nor the secret.
fn refused(s: &str) -> String {
    match Url::parse(s) {
        Err(Error::Protocol(reason)) => {
            assert!(!reason.contains(SECRET), "{s:?}: {reason}");
            if s.len() > 8 {
                assert!(!reason.contains(s), "{s:?}: {reason}");
            }
            reason
        }
        other => panic!("{s:?} gave {other:?}"),
    }
}

fn join(base: &str, location: &str) -> String {
    parse(base).join(location).unwrap_or_else(|e| panic!("{base:?} + {location:?} was refused: {e}")).full()
}

fn join_refused(base: &str, location: &str) -> String {
    match parse(base).join(location) {
        Err(Error::Protocol(reason)) => {
            assert!(!reason.contains(SECRET), "{location:?}: {reason}");
            if location.len() > 8 {
                assert!(!reason.contains(location), "{location:?}: {reason}");
            }
            reason
        }
        other => panic!("{base:?} + {location:?} gave {other:?}"),
    }
}

fn url(https: bool, host: &str, port: u16, path: &str) -> Url {
    Url { https, host: host.to_owned(), port, path: path.to_owned() }
}

// ---- default_port -------------------------------------------------------------------------------------------

#[test]
fn default_ports() {
    assert_eq!(default_port(true), 443);
    assert_eq!(default_port(false), 80);
}

// ---- parse: valid addresses -----------------------------------------------------------------------------------

#[test]
fn parses_the_parts() {
    assert_eq!(parse("https://jmap.example.com/session"), url(true, "jmap.example.com", 443, "/session"));
    assert_eq!(parse("http://example.com/a/b?c=d&e"), url(false, "example.com", 80, "/a/b?c=d&e"));
}

#[test]
fn scheme_and_host_are_case_insensitive_path_is_not() {
    assert_eq!(parse("HTTPS://Mail.EXAMPLE.com/Path?Q=A"), url(true, "mail.example.com", 443, "/Path?Q=A"));
    assert_eq!(parse("hTtP://H").host, "h");
    assert!(!parse("HtTp://h").https);
    assert!(parse("hTTpS://h").https);
}

#[test]
fn ports() {
    assert_eq!(parse("https://h:8443/x").port, 8443);
    assert_eq!(parse("http://h:8080").port, 8080);
    assert_eq!(parse("https://h:1/").port, 1);
    assert_eq!(parse("https://h:65535/").port, 65535);
    assert_eq!(parse("https://h:443/").port, 443);
    assert_eq!(parse("http://h:443/").port, 443);
    assert_eq!(parse("https://h:80/").port, 80);
    assert_eq!(parse("https://h:00080/").port, 80);
    // `:` with nothing after it means the default port.
    assert_eq!(parse("https://h:/x").port, 443);
    assert_eq!(parse("http://h:").port, 80);
    assert_eq!(parse("https://h:?q").port, 443);
}

#[test]
fn bad_ports() {
    for bad in [
        "https://h:0/",
        "https://h:00000/",
        "https://h:65536/",
        "https://h:99999/",
        "https://h:123456/",
        "https://h:-1/",
        "https://h:+80/",
        "https://h:8a/",
        "https://h:0x50/",
        "https://h:80:81/",
        "https://h::80/",
    ] {
        refused(bad);
    }
}

#[test]
fn path_gets_a_leading_slash() {
    assert_eq!(parse("https://h").path, "/");
    assert_eq!(parse("https://h:8443").path, "/");
    assert_eq!(parse("https://h?x=1").path, "/?x=1");
    assert_eq!(parse("https://h:1?x").path, "/?x");
    assert_eq!(parse("https://h/").path, "/");
}

#[test]
fn an_empty_query_is_kept() {
    assert_eq!(parse("https://h?").path, "/?");
    assert_eq!(parse("https://h/a?").path, "/a?");
}

#[test]
fn fragment_is_dropped() {
    assert_eq!(parse("https://h/p#frag").path, "/p");
    assert_eq!(parse("https://h#frag").path, "/");
    assert_eq!(parse("https://h?q#f").path, "/?q");
    assert_eq!(parse("https://h/p?q#f?x#y").path, "/p?q");
    assert_eq!(parse("https://h:8443#").path, "/");
    assert_eq!(parse("https://h/#/a/b").path, "/");
}

#[test]
fn dot_segments_are_removed_from_the_path_not_the_query() {
    assert_eq!(parse("https://h/a/../b").path, "/b");
    assert_eq!(parse("https://h/a/./b/").path, "/a/b/");
    assert_eq!(parse("https://h/../../x").path, "/x");
    assert_eq!(parse("https://h/a/%2E%2e/b").path, "/b");
    assert_eq!(parse("https://h/a/b/..").path, "/a/");
    assert_eq!(parse("https://h/a?x=/../y").path, "/a?x=/../y");
    assert_eq!(parse("https://h/a/...").path, "/a/...");
}

#[test]
fn other_path_characters_are_kept_as_they_are() {
    assert_eq!(parse("https://h/a%20b/c%2Fd;e=f,g@h!$&'()*+:").path, "/a%20b/c%2Fd;e=f,g@h!$&'()*+:");
    assert_eq!(parse("https://h/é?ü").path, "/é?ü");
    // An `@` after the authority is not user information.
    assert_eq!(parse("https://h/@x").host, "h");
    assert_eq!(parse("https://h?a@b").host, "h");
    assert_eq!(parse("https://h#a@b").host, "h");
}

#[test]
fn length_limit() {
    let prefix = "https://h/";
    let at_limit = format!("{prefix}{}", "a".repeat(8192 - prefix.len()));
    assert_eq!(at_limit.len(), 8192);
    parse(&at_limit);
    let over = format!("{at_limit}a");
    refused(&over);
}

// ---- parse: whitespace, controls, schemes -------------------------------------------------------------------------

#[test]
fn whitespace_and_controls_are_refused_anywhere() {
    for bad in [
        " https://h/",
        "https://h/ ",
        "\thttps://h/",
        "https://h/\n",
        "https://h/\r\n",
        "https://h/a b",
        "https://h /",
        "https:// h/",
        "https://h/a\tb",
        "https://h/\u{0}",
        "https://h/\u{1b}",
        "https://h/\u{7f}",
        "https://h/\u{85}",
        "https://h/\u{a0}",
        "https://h/\u{3000}",
        "https://h/#a b",
        "https://h/?a\u{c}b",
    ] {
        refused(bad);
    }
}

#[test]
fn backslashes_are_refused() {
    refused("https://h\\evil.example/");
    refused("https://h/a\\..\\b");
    refused("https:\\\\h/");
}

#[test]
fn only_http_and_https() {
    for bad in [
        "",
        "h",
        "//h/",
        "/path",
        "ftp://h/",
        "ws://h/",
        "wss://h/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/plain,x",
        "mailto:a@h",
        "httpx://h/",
        "http:h",
        "http:/h",
        "https:",
        "https:/",
        "https//h",
        "http;//h",
        "xhttps://h",
    ] {
        refused(bad);
    }
}

#[test]
fn user_information_is_refused() {
    for bad in ["https://user@h/", &format!("https://user:{SECRET}@h/"), "https://@h/", "https://h@/", "https://a@b@h/", "https://user@[::1]/"] {
        refused(bad);
    }
}

#[test]
fn an_empty_host_is_refused() {
    for bad in ["https://", "https:///x", "https://:443/", "https://?x", "https://#x", "https://:"] {
        refused(bad);
    }
}

#[test]
fn host_length_limit() {
    let at_limit = "a".repeat(253);
    assert_eq!(parse(&format!("https://{at_limit}/")).host, at_limit);
    let labels = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61));
    assert_eq!(labels.len(), 253);
    parse(&format!("https://{labels}:8443/"));
    refused(&format!("https://{}/", "a".repeat(254)));
    refused(&format!("https://{labels}x/"));
}

#[test]
fn host_characters() {
    assert_eq!(parse("https://a-b.c-d.example/").host, "a-b.c-d.example");
    assert_eq!(parse("https://xn--bcher-kva.example/").host, "xn--bcher-kva.example");
    assert_eq!(parse("https://localhost:8080/").host, "localhost");
    assert_eq!(parse("https://123abc.example/").host, "123abc.example");
    for bad in [
        "https://exa_mple.com/",
        "https://ex%41mple.com/",
        "https://exämple.com/",
        "https://bücher.example/",
        "https://ex!ample.com/",
        "https://ex*ample.com/",
        "https://ex,ample.com/",
        "https://ex~ample.com/",
        "https://ex]ample.com/",
        "https://ex[ample.com/",
        "https://ex\"ample.com/",
        "https://ex<ample.com/",
    ] {
        refused(bad);
    }
}

// ---- parse: IP addresses ---------------------------------------------------------------------------------------

#[test]
fn ipv4_hosts() {
    assert_eq!(parse("http://127.0.0.1:8080/jmap"), url(false, "127.0.0.1", 8080, "/jmap"));
    assert_eq!(parse("https://192.168.1.10/").host, "192.168.1.10");
    assert_eq!(parse("https://0.0.0.0/").host, "0.0.0.0");
    assert_eq!(parse("https://255.255.255.255/").host, "255.255.255.255");
}

#[test]
fn numbers_a_browser_would_read_another_way_are_refused() {
    for bad in [
        "http://127.1/",
        "http://127.0.1/",
        "http://2130706433/",
        "http://0x7f.0.0.1/",
        "http://0x7f000001/",
        "http://010.0.0.1/",
        "http://127.0.0.01/",
        "http://1.2.3.256/",
        "http://1.2.3.4.5/",
        "http://1.2.3.4./",
        "http://example.123/",
        "http://example.0x1f/",
        "http://.1.2.3.4/",
    ] {
        refused(bad);
    }
}

#[test]
fn ipv6_hosts_keep_their_brackets_and_are_lower_cased() {
    assert_eq!(parse("https://[::1]/"), url(true, "[::1]", 443, "/"));
    assert_eq!(parse("http://[::1]:8080"), url(false, "[::1]", 8080, "/"));
    assert_eq!(parse("https://[2001:DB8::A]/x").host, "[2001:db8::a]");
    assert_eq!(parse("https://[::]/").host, "[::]");
    assert_eq!(parse("https://[1:2:3:4:5:6:7:8]:1/").host, "[1:2:3:4:5:6:7:8]");
    assert_eq!(parse("https://[::ffff:192.0.2.1]/").host, "[::ffff:192.0.2.1]");
    assert_eq!(parse("https://[::1]:/").port, 443);
    assert_eq!(parse("https://[::1]?q").path, "/?q");
    assert_eq!(parse("https://[::1]#f").path, "/");
}

#[test]
fn bad_ipv6_hosts() {
    for bad in [
        "https://[::1/",
        "https://[]/",
        "https://[/",
        "https://::1/",
        "https://[::1]x/",
        "https://[::1]]/",
        "https://[::1]:x/",
        "https://[[::1]]/",
        "https://[zz::1]/",
        "https://[fe80::1%25eth0]/",
        "https://[v1.fe]/",
        "https://[1:2]/",
        "https://[1:2:3:4:5:6:7:8:9]/",
        "https://[1::2::3]/",
        "https://[12345::]/",
        "https://[::1.2.3]/",
        "https://[127.0.0.1]/",
        "https://[:::]/",
        "https://[ ::1]/",
    ] {
        refused(bad);
    }
}

// ---- join ---------------------------------------------------------------------------------------------------------

/// RFC 3986 §5.4.1, normal examples (the `g:h` example is another scheme: see `join_refuses_other_schemes`).
#[test]
fn join_rfc3986_normal_examples() {
    let base = "http://a/b/c/d;p?q";
    for (reference, expected) in [
        ("g", "http://a/b/c/g"),
        ("./g", "http://a/b/c/g"),
        ("g/", "http://a/b/c/g/"),
        ("/g", "http://a/g"),
        ("//g", "http://g/"),
        ("?y", "http://a/b/c/d;p?y"),
        ("g?y", "http://a/b/c/g?y"),
        ("#s", "http://a/b/c/d;p?q"),
        ("g#s", "http://a/b/c/g"),
        ("g?y#s", "http://a/b/c/g?y"),
        (";x", "http://a/b/c/;x"),
        ("g;x", "http://a/b/c/g;x"),
        ("g;x?y#s", "http://a/b/c/g;x?y"),
        ("", "http://a/b/c/d;p?q"),
        (".", "http://a/b/c/"),
        ("./", "http://a/b/c/"),
        ("..", "http://a/b/"),
        ("../", "http://a/b/"),
        ("../g", "http://a/b/g"),
        ("../..", "http://a/"),
        ("../../", "http://a/"),
        ("../../g", "http://a/g"),
    ] {
        assert_eq!(join(base, reference), expected, "{reference:?}");
    }
}

/// RFC 3986 §5.4.2, abnormal examples.
#[test]
fn join_rfc3986_abnormal_examples() {
    let base = "http://a/b/c/d;p?q";
    for (reference, expected) in [
        ("../../../g", "http://a/g"),
        ("../../../../g", "http://a/g"),
        ("/./g", "http://a/g"),
        ("/../g", "http://a/g"),
        ("g.", "http://a/b/c/g."),
        (".g", "http://a/b/c/.g"),
        ("g..", "http://a/b/c/g.."),
        ("..g", "http://a/b/c/..g"),
        ("./../g", "http://a/b/g"),
        ("./g/.", "http://a/b/c/g/"),
        ("g/./h", "http://a/b/c/g/h"),
        ("g/../h", "http://a/b/c/h"),
        ("g;x=1/./y", "http://a/b/c/g;x=1/y"),
        ("g;x=1/../y", "http://a/b/c/y"),
        ("g?y/./x", "http://a/b/c/g?y/./x"),
        ("g?y/../x", "http://a/b/c/g?y/../x"),
        ("g#s/./x", "http://a/b/c/g"),
        ("g#s/../x", "http://a/b/c/g"),
    ] {
        assert_eq!(join(base, reference), expected, "{reference:?}");
    }
    // A strict parser keeps `http:g` as an absolute reference, which is not a valid http address.
    join_refused(base, "http:g");
}

#[test]
fn join_absolute_addresses() {
    let base = "https://a.example:8443/b/c";
    assert_eq!(join(base, "https://other.example/x#f"), "https://other.example/x");
    assert_eq!(join(base, "HTTP://Other.Example:81/Y?z"), "http://other.example:81/Y?z");
    assert_eq!(join(base, "http://a.example:8443/"), "http://a.example:8443/");
    assert_eq!(join(base, "https://[::1]/a/../b"), "https://[::1]/b");
    join_refused(base, "https://user@other.example/");
    join_refused(base, "https:///x");
    join_refused(base, "https://h:0/");
}

#[test]
fn join_refuses_other_schemes() {
    let base = "https://a.example/b/c";
    for bad in [
        "g:h",
        "ftp://h/",
        "ws://h/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<b>x</b>",
        "mailto:a@b.example",
        "c:/windows",
        "a+b.c-d:x",
        &format!("custom-scheme:{SECRET}"),
    ] {
        join_refused(base, bad);
    }
}

#[test]
fn join_network_path_keeps_the_scheme() {
    assert_eq!(join("https://a.example:8443/b", "//other.example/x?y#z"), "https://other.example/x?y");
    assert_eq!(join("http://a/b", "//other:81"), "http://other:81/");
    assert_eq!(join("https://a/b", "//[::1]:9/p"), "https://[::1]:9/p");
    join_refused("https://a/b", "//user@other/");
    join_refused("https://a/b", "//");
    join_refused("https://a/b", "///x");
}

#[test]
fn join_absolute_path_keeps_scheme_host_and_port() {
    let base = "https://a.example:8443/b/c?q";
    assert_eq!(join(base, "/x"), "https://a.example:8443/x");
    assert_eq!(join(base, "/x?y=1#frag"), "https://a.example:8443/x?y=1");
    assert_eq!(join(base, "/"), "https://a.example:8443/");
    assert_eq!(join(base, "/a/b/../c"), "https://a.example:8443/a/c");
    assert_eq!(join("http://[::1]:8080/a", "/z"), "http://[::1]:8080/z");
}

#[test]
fn join_query_replaces_the_query_only() {
    assert_eq!(join("https://a/b/c?old", "?new"), "https://a/b/c?new");
    assert_eq!(join("https://a/b/c", "?new#f"), "https://a/b/c?new");
    assert_eq!(join("https://a/b/c?old", "?"), "https://a/b/c?");
    assert_eq!(join("https://a:9/?old", "?n=/../x"), "https://a:9/?n=/../x");
}

#[test]
fn join_empty_gives_the_address_back() {
    let base = parse("https://a:9/b/c?q#frag");
    assert_eq!(base.join("").unwrap(), base);
    assert_eq!(base.join("#other").unwrap(), base);
}

#[test]
fn join_relative_paths() {
    assert_eq!(join("https://a/", "g"), "https://a/g");
    assert_eq!(join("https://a", "g"), "https://a/g");
    assert_eq!(join("https://a/b/", "g"), "https://a/b/g");
    assert_eq!(join("https://a/b?x/y/z", "g"), "https://a/g");
    assert_eq!(join("https://a:8443/b/c/d", "../../../../x"), "https://a:8443/x");
    assert_eq!(join("https://a/b/c", "%2e%2e/x"), "https://a/x");
    assert_eq!(join("https://[::1]:8443/a/b", "c?d"), "https://[::1]:8443/a/c?d");
    // A first segment holding `:` reads as a scheme (RFC 3986 §4.2): write `./x:y` for a path.
    join_refused("https://a/b/c", "x:y/../z");
    assert_eq!(join("https://a/b/c", "./x:y/../z"), "https://a/b/z");
}

#[test]
fn join_validates_the_reference_text() {
    let base = "https://a/b/c";
    for bad in [" g", "g ", "g h", "g\n", "\\\\evil.example/x", "/\\evil.example", "g\u{7f}"] {
        join_refused(base, bad);
    }
    let long = "g".repeat(8193);
    join_refused(base, &long);
    // The joined address is held to the same length limit.
    let almost = "g".repeat(8190);
    join_refused(base, &almost);
}

#[test]
fn join_guards_a_hand_built_path_without_slash() {
    let odd = url(true, "a.example", 443, "x");
    assert_eq!(odd.join("").unwrap(), parse("https://a.example/x"));
    assert_eq!(odd.join("?q").unwrap(), parse("https://a.example/x?q"));
    assert_eq!(odd.join("g").unwrap(), parse("https://a.example/g"));
    assert_eq!(odd.join("g").unwrap().host, "a.example");
}

// ---- origin, full, round trips --------------------------------------------------------------------------------------

#[test]
fn origin_shows_the_port_only_when_not_the_default() {
    assert_eq!(parse("https://h:443/x").origin(), "https://h");
    assert_eq!(parse("http://h:80/x").origin(), "http://h");
    assert_eq!(parse("http://h:443/x").origin(), "http://h:443");
    assert_eq!(parse("https://h:80/x").origin(), "https://h:80");
    assert_eq!(parse("https://H:8443/x?y").origin(), "https://h:8443");
    assert_eq!(parse("https://[::1]:8443/").origin(), "https://[::1]:8443");
    assert_eq!(parse("https://[::1]/").origin(), "https://[::1]");
}

#[test]
fn full_is_origin_then_path() {
    assert_eq!(parse("HTTPS://H:443/a?b#c").full(), "https://h/a?b");
    assert_eq!(parse("http://h:8080").full(), "http://h:8080/");
    assert_eq!(parse("https://h?q").full(), "https://h/?q");
}

#[test]
fn full_parses_back_to_an_equal_url() {
    for address in [
        "https://h",
        "http://h:80",
        "http://h:443/x",
        "https://h:/a/../b?c=/../d#e",
        "HTTPS://Mail.Example.COM:8443/jmap/api/?x=1&y=%20#f",
        "https://[2001:DB8::1]:1/",
        "http://127.0.0.1:65535/a%2Fb",
        "https://h?",
        "https://h/é",
    ] {
        let parsed = parse(address);
        assert_eq!(parse(&parsed.full()), parsed, "{address:?}");
        assert_eq!(parse(&parsed.full()).full(), parsed.full(), "{address:?}");
    }
}

// ---- is_loopback ----------------------------------------------------------------------------------------------------

#[test]
fn loopback_hosts() {
    for address in [
        "http://localhost/",
        "http://LOCALHOST:8080/",
        "http://jmap.localhost/",
        "http://a.b.localhost/",
        "http://127.0.0.1/",
        "http://127.0.0.0/",
        "http://127.1.2.3/",
        "http://127.255.255.255/",
        "http://[::1]/",
        "http://[::1]:8080/",
        "http://[0:0:0:0:0:0:0:1]/",
        "http://[::0:1]/",
        "http://[0::1]/",
    ] {
        assert!(parse(address).is_loopback(), "{address:?}");
    }
}

#[test]
fn not_loopback_hosts() {
    for address in [
        "http://localhost.example.com/",
        "http://localhostx/",
        "http://notlocalhost/",
        "http://my-localhost/",
        "http://128.0.0.1/",
        "http://126.255.255.255/",
        "http://10.0.0.1/",
        "http://0.0.0.0/",
        "http://[::]/",
        "http://[::2]/",
        "http://[::1:0]/",
        "http://[::ffff:127.0.0.1]/",
        "http://example.com/",
    ] {
        assert!(!parse(address).is_loopback(), "{address:?}");
    }
}

#[test]
fn loopback_of_hand_built_hosts_needs_four_plain_decimal_numbers() {
    for host in ["127.000.0.1", "0127.0.0.1", "127.1", "127.0.0.1.", "127.0.0.256", "2130706433", "[::1", "::1"] {
        assert!(!url(false, host, 80, "/").is_loopback(), "{host:?}");
    }
    assert!(url(false, "127.9.9.9", 80, "/").is_loopback());
}

// ---- Debug -------------------------------------------------------------------------------------------------------------

#[test]
fn debug_shows_the_origin_only() {
    let address = parse(&format!("https://h.example:8443/download/{SECRET}?access_token={SECRET}"));
    let shown = format!("{address:?}");
    assert_eq!(shown, "https://h.example:8443");
    assert!(!shown.contains(SECRET));
    assert_eq!(format!("{:?}", parse("http://[::1]/x")), "http://[::1]");
    assert_eq!(format!("{:#?}", parse("https://h/p?q")), "https://h");
}

// ---- pct ---------------------------------------------------------------------------------------------------------------

#[test]
fn pct_keeps_unreserved_characters() {
    let unreserved = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    assert_eq!(pct(unreserved), unreserved);
    assert_eq!(pct(""), "");
}

#[test]
fn pct_encodes_everything_else_with_upper_case_hex() {
    assert_eq!(pct(" "), "%20");
    assert_eq!(pct("a b"), "a%20b");
    assert_eq!(pct("/"), "%2F");
    assert_eq!(pct("a/b"), "a%2Fb");
    assert_eq!(pct("%"), "%25");
    assert_eq!(pct("+"), "%2B");
    assert_eq!(pct(":/?#[]@!$&'()*+,;="), "%3A%2F%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D");
    assert_eq!(pct("\"<>\\^`{|}"), "%22%3C%3E%5C%5E%60%7B%7C%7D");
    assert_eq!(pct("\n\t\u{0}\u{7f}"), "%0A%09%00%7F");
}

#[test]
fn pct_encodes_utf8_bytes() {
    assert_eq!(pct("é"), "%C3%A9");
    assert_eq!(pct("Grüße.pdf"), "Gr%C3%BC%C3%9Fe.pdf");
    assert_eq!(pct("日本"), "%E6%97%A5%E6%9C%AC");
    assert_eq!(pct("😀"), "%F0%9F%98%80");
}

/// RFC 6570 §3.2.2 examples of simple string expansion.
#[test]
fn pct_matches_rfc6570_simple_expansion() {
    assert_eq!(pct("value"), "value");
    assert_eq!(pct("Hello World!"), "Hello%20World%21");
    assert_eq!(pct("50%"), "50%25");
}

#[test]
fn pct_fills_a_jmap_download_template() {
    let template = "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}";
    let filled = template
        .replace("{accountId}", &pct("u1"))
        .replace("{blobId}", &pct("Gabc/def"))
        .replace("{name}", &pct("rapport final.pdf"))
        .replace("{type}", &pct("application/pdf"));
    assert_eq!(filled, "https://jmap.example.com/download/u1/Gabc%2Fdef/rapport%20final.pdf?accept=application%2Fpdf");
    assert_eq!(parse(&filled).path, "/download/u1/Gabc%2Fdef/rapport%20final.pdf?accept=application%2Fpdf");
}

// ---- pct_decode --------------------------------------------------------------------------------------------------------

#[test]
fn pct_decode_valid_sequences_either_case() {
    assert_eq!(pct_decode("%41%42c"), "ABc");
    assert_eq!(pct_decode("%C3%A9"), "é");
    assert_eq!(pct_decode("%c3%a9"), "é");
    assert_eq!(pct_decode("%c3%A9"), "é");
    assert_eq!(pct_decode("a%20b"), "a b");
    assert_eq!(pct_decode("%2B"), "+");
    assert_eq!(pct_decode("%25"), "%");
    assert_eq!(pct_decode("%2525"), "%25");
    assert_eq!(pct_decode("%00"), "\u{0}");
    assert_eq!(pct_decode(""), "");
    assert_eq!(pct_decode("plain"), "plain");
}

#[test]
fn pct_decode_leaves_other_percent_signs() {
    assert_eq!(pct_decode("100%"), "100%");
    assert_eq!(pct_decode("%"), "%");
    assert_eq!(pct_decode("%4"), "%4");
    assert_eq!(pct_decode("%zz"), "%zz");
    assert_eq!(pct_decode("%G1"), "%G1");
    assert_eq!(pct_decode("%4g"), "%4g");
    assert_eq!(pct_decode("%%41"), "%A");
    assert_eq!(pct_decode("%4%41"), "%4A");
    assert_eq!(pct_decode("é%"), "é%");
    assert_eq!(pct_decode("%é"), "%é");
}

#[test]
fn pct_decode_keeps_plus() {
    assert_eq!(pct_decode("a+b"), "a+b");
    assert_eq!(pct_decode("+%20+"), "+ +");
}

#[test]
fn pct_decode_replaces_invalid_utf8() {
    assert_eq!(pct_decode("%E9"), "\u{FFFD}");
    assert_eq!(pct_decode("a%FFb"), "a\u{FFFD}b");
    assert_eq!(pct_decode("%C3"), "\u{FFFD}");
    assert_eq!(pct_decode("%C3%28"), "\u{FFFD}(");
}

#[test]
fn pct_decode_reverses_pct() {
    for text in ["", "a b", "Grüße/€?#&=+%", "😀 日本", "\u{0}\n", "%41"] {
        assert_eq!(pct_decode(&pct(text)), text, "{text:?}");
    }
}
