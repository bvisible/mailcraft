// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! What the interface looks up by its English text, and what the catalogs must and must not hold of it.
//!
//! The strings are found where the code writes them: every `tl!` call in the workspace's sources
//! (its literal, the text of a constant it names, or the text inside a longer argument), and the
//! label and the location of every command, which the window shows through [`super::t`] and
//! [`super::location`]. Nothing is run and nothing is written.
//!
//! Some strings reach `tl!` from somewhere else: a ribbon button's label that a table hands on,
//! the name of a month. The scan can't follow them, so it doesn't list them for translators and a
//! catalog isn't required to have them; a catalog may have them all the same, since the code has
//! them as strings ([`stale_entries`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::LOCATION_SEPARATOR;
use super::catalog::{self, LANGUAGES};

/// The window commands that command search lists with the engine's (`dialogs::all_commands`).
const SEARCHED_WINDOW_COMMANDS: [&str; 5] = ["ui.sync", "ui.open", "ui.import", "ui.attach", "ui.collapseRibbon"];

/// A string of the interface, and where it is written: `path:line` in the sources, or a command's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub english: String,
    pub origin: String,
}

/// Whether a string has any text to translate in it (one with no letter at all, `…` or `1`, has not).
pub fn is_text(english: &str) -> bool {
    english.chars().any(char::is_alphabetic)
}

/// What the scan of the workspace found, once for all the tests.
fn collected() -> &'static Collected {
    static COLLECTED: OnceLock<Collected> = OnceLock::new();
    COLLECTED.get_or_init(|| {
        let collected = collect(&workspace_root());
        assert!(collected.unresolved.is_empty(), "constants that tl! names and that no `&str` constant defines: {:?}", collected.unresolved);
        collected
    })
}

/// Every string of the interface, each once, in the order the sources and the command tables have them.
pub fn interface_strings() -> &'static [Found] {
    &collected().found
}

/// What is wrong with the entries of a catalog's text, as the lines of its entries that are for a
/// string the code doesn't have: one that was renamed or removed, whose entry is dead. A string
/// is the code's when the scan finds it looked up, or when the code has it as a string at all,
/// which is how a label that a table hands to `tl!` (a ribbon button's, a month's name) is
/// found, as the scan can't follow the table (nor the abbreviations of months that are cut from
/// their names).
pub fn stale_entries(source: &str) -> Vec<String> {
    let looked_up: HashSet<&str> = interface_strings().iter().map(|f| f.english.as_str()).collect();
    catalog::parse(source)
        .entries
        .iter()
        .filter(|entry| !looked_up.contains(entry.english.as_str()) && !collected().literals.contains(&entry.english))
        .map(|entry| format!("line {}: {:?}", entry.line, entry.english))
        .collect()
}

/// The strings of the interface that a catalog's text has no plain entry for (a context isn't
/// asked for by `tl!`), leaving out those with no text in them.
pub fn missing_entries(source: &str) -> Vec<&'static str> {
    let have: HashSet<String> =
        catalog::parse(source).entries.into_iter().filter(|entry| entry.context.is_empty()).map(|entry| entry.english).collect();
    interface_strings().iter().filter(|f| is_text(&f.english) && !have.contains(&f.english)).map(|f| f.english.as_str()).collect()
}

// ---------------------------------------------------------------------------------------------
// Reading Rust source

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// A string literal's text, as the compiler reads it.
    Str(String),
    Punct(char),
    /// A number, a character or a lifetime.
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    /// Where it starts, from 1.
    line: usize,
}

/// The tokens of Rust source: enough to find macro calls and constants without being misled by
/// comments, strings, characters or lifetimes.
fn lex(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut tokens = Vec::new();
    let (mut i, mut line) = (0, 1);
    while let Some(c) = at(i) {
        let start = line;
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while at(i).is_some_and(|c| c != '\n') {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            // Block comments nest.
            let mut depth = 1;
            i += 2;
            while depth > 0 {
                match (at(i), at(i + 1)) {
                    (None, _) => break,
                    (Some('/'), Some('*')) => {
                        depth += 1;
                        i += 2;
                    }
                    (Some('*'), Some('/')) => {
                        depth -= 1;
                        i += 2;
                    }
                    (Some('\n'), _) => {
                        line += 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        } else if let Some((hashes, quote)) = raw_string_start(&chars, i) {
            let (text, next) = read_raw_string(&chars, quote + 1, hashes, &mut line);
            tokens.push(Token { tok: Tok::Str(text), line: start });
            i = next;
        } else if c == '"' || (c == 'b' && at(i + 1) == Some('"')) {
            let (text, next) = read_string(&chars, if c == '"' { i + 1 } else { i + 2 }, &mut line);
            tokens.push(Token { tok: Tok::Str(text), line: start });
            i = next;
        } else if c == 'b' && at(i + 1) == Some('\'') {
            // A byte: the character after it is read as any.
            i += 1;
        } else if c == '\'' {
            // A character ('a', '"', '\n', '\u{2026}') or a lifetime ('a).
            if at(i + 1) == Some('\\') {
                i += 3;
                while at(i).is_some_and(|c| c != '\'') {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == Some('\'') {
                i += 3;
            } else {
                i += 1;
                while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    i += 1;
                }
            }
            tokens.push(Token { tok: Tok::Other, line: start });
        } else if c.is_alphabetic() || c == '_' {
            let mut name = String::new();
            while let Some(c) = at(i).filter(|c| c.is_alphanumeric() || *c == '_') {
                name.push(c);
                i += 1;
            }
            tokens.push(Token { tok: Tok::Ident(name), line: start });
        } else if c.is_ascii_digit() {
            while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                i += 1;
            }
            tokens.push(Token { tok: Tok::Other, line: start });
        } else {
            tokens.push(Token { tok: Tok::Punct(c), line: start });
            i += 1;
        }
    }
    tokens
}

/// If a raw string (`r"…"`, `r#"…"#`, `br"…"`) starts at `i`: how many `#` it has, and where its opening quote is.
fn raw_string_start(chars: &[char], i: usize) -> Option<(usize, usize)> {
    let mut at = i;
    if chars.get(at) == Some(&'b') {
        at += 1;
    }
    if chars.get(at) != Some(&'r') {
        return None;
    }
    at += 1;
    let mut hashes = 0;
    while chars.get(at) == Some(&'#') {
        hashes += 1;
        at += 1;
    }
    (chars.get(at) == Some(&'"')).then_some((hashes, at))
}

/// The text of a raw string whose text starts at `i`, and where the source goes on after it.
fn read_raw_string(chars: &[char], mut i: usize, hashes: usize, line: &mut usize) -> (String, usize) {
    let mut text = String::new();
    while let Some(&c) = chars.get(i) {
        i += 1;
        if c == '"' && (0..hashes).all(|n| chars.get(i + n) == Some(&'#')) {
            return (text, i + hashes);
        }
        if c == '\n' {
            *line += 1;
        }
        text.push(c);
    }
    (text, i)
}

/// The text of a string whose text starts at `i`, and where the source goes on after it.
fn read_string(chars: &[char], mut i: usize, line: &mut usize) -> (String, usize) {
    let mut text = String::new();
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '"' => return (text, i),
            '\\' => {
                let Some(&escaped) = chars.get(i) else { break };
                i += 1;
                match escaped {
                    'n' => text.push('\n'),
                    'r' => text.push('\r'),
                    't' => text.push('\t'),
                    '0' => text.push('\0'),
                    'x' => {
                        let hex: String = chars.iter().skip(i).take(2).collect();
                        i += 2;
                        text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    'u' => {
                        let mut hex = String::new();
                        if chars.get(i) == Some(&'{') {
                            i += 1;
                            while let Some(&h) = chars.get(i) {
                                i += 1;
                                if h == '}' {
                                    break;
                                }
                                if h != '_' {
                                    hex.push(h);
                                }
                            }
                        }
                        text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    // A backslash at the end of a line leaves out the new line and the indentation of the next.
                    '\n' => {
                        *line += 1;
                        while let Some(&white) = chars.get(i).filter(|c| c.is_whitespace()) {
                            if white == '\n' {
                                *line += 1;
                            }
                            i += 1;
                        }
                    }
                    other => text.push(other),
                }
            }
            c => {
                if c == '\n' {
                    *line += 1;
                }
                text.push(c);
            }
        }
    }
    (text, i)
}

/// The index of the delimiter that closes the one at `open`.
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, token) in tokens.get(open..)?.iter().enumerate() {
        match token.tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// What one `tl!` call looks up.
#[derive(Clone, Debug, PartialEq)]
enum Lookup {
    /// The text of a literal, which is the whole argument or part of one (`tl!(x.unwrap_or("text"))`), and its line.
    Text(String, usize),
    /// A constant that the argument names (`tl!(NAME)`, `tl!(module::NAME)`), and its line.
    Constant(String, usize),
}

/// What a source file looks up and defines.
#[derive(Debug, Default, PartialEq)]
struct Scan {
    /// Each `tl!` call's, in order.
    lookups: Vec<Lookup>,
    /// The `&str` constants and statics defined: name, text, line.
    constants: Vec<(String, String, usize)>,
    /// Every string literal of the source (a label that a table hands to `tl!` is one).
    strings: Vec<String>,
}

fn scan(source: &str) -> Scan {
    let tokens = lex(source);
    let mut scan = Scan::default();
    for (k, token) in tokens.iter().enumerate() {
        let word = match &token.tok {
            Tok::Ident(word) => word,
            Tok::Str(text) => {
                if !text.is_empty() {
                    scan.strings.push(text.clone());
                }
                continue;
            }
            _ => continue,
        };
        match word.as_str() {
            // (`run_word!` marks a sentence of the engine's that the window reads back: a lookup like any other.)
            "tl" | "run_word" => {
                let (Some(bang), Some(open)) = (tokens.get(k + 1), tokens.get(k + 2)) else { continue };
                if bang.tok != Tok::Punct('!') || !matches!(open.tok, Tok::Punct('(' | '[' | '{')) {
                    continue;
                }
                let Some(close) = closing(&tokens, k + 2) else { continue };
                scan.lookups.extend(lookups_in(tokens.get(k + 3..close).unwrap_or_default()));
            }
            "const" | "static" => scan.constants.extend(constant_at(&tokens, k)),
            _ => {}
        }
    }
    scan
}

/// A string literal of a source file, as [`literals_of`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Literal {
    /// Its text, as the compiler reads it.
    pub text: String,
    /// Where it starts, from 1.
    pub line: usize,
    /// Whether it is an argument of a `log::…!` call: words for the log, not for a person.
    pub logged: bool,
}

/// The string literals of Rust source that the program can show (the census of the sentences that `run_words` reads back
/// holds them to its rows): every literal but those of a `#[cfg(test)]` module written inline, which are the tests'.
pub fn literals_of(source: &str) -> Vec<Literal> {
    let tokens = lex(source);
    let mut out = Vec::new();
    let mut k = 0;
    // Where the arguments of a `log::…!` call end (the literals before are logged).
    let mut logged_until = 0;
    while let Some(token) = tokens.get(k) {
        let is = |n: usize, tok: Tok| tokens.get(k + n).is_some_and(|t| t.tok == tok);
        let ident = |n: usize, name: &str| is(n, Tok::Ident(name.into()));
        if is(0, Tok::Punct('#')) && is(1, Tok::Punct('[')) && ident(2, "cfg") && is(3, Tok::Punct('(')) && ident(4, "test") && is(5, Tok::Punct(')'))
        {
            // `#[cfg(test)] mod name { … }` is skipped whole; `#[cfg(test)] mod name;` names a file of its own.
            if ident(7, "mod")
                && is(9, Tok::Punct('{'))
                && let Some(close) = closing(&tokens, k + 9)
            {
                k = close + 1;
                continue;
            }
        }
        if ident(0, "log")
            && is(1, Tok::Punct(':'))
            && is(2, Tok::Punct(':'))
            && is(4, Tok::Punct('!'))
            && let Some(close) = closing(&tokens, k + 5)
        {
            logged_until = logged_until.max(close);
        }
        if let Tok::Str(text) = &token.tok
            && !text.is_empty()
        {
            out.push(Literal { text: text.clone(), line: token.line, logged: k < logged_until });
        }
        k += 1;
    }
    out
}

/// What the argument of a `tl!` call looks up.
fn lookups_in(args: &[Token]) -> Vec<Lookup> {
    if let [Token { tok: Tok::Str(text), line }] = args {
        return if text.is_empty() { Vec::new() } else { vec![Lookup::Text(text.clone(), *line)] };
    }
    // The names of a path, the last in capitals: a constant.
    let path =
        args.iter().all(|t| matches!(t.tok, Tok::Ident(_) | Tok::Punct(':'))) && matches!(args.first(), Some(Token { tok: Tok::Ident(_), .. }));
    if let (true, Some(Token { tok: Tok::Ident(name), line })) = (path, args.last())
        && name.chars().any(|c| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return vec![Lookup::Constant(name.clone(), *line)];
    }
    // Anything else looks up what it is made of that is text.
    args.iter()
        .filter_map(|t| match &t.tok {
            Tok::Str(text) if !text.is_empty() => Some(Lookup::Text(text.clone(), t.line)),
            _ => None,
        })
        .collect()
}

/// The `&str` constant that `const` or `static` at `k` defines: `const NAME: &str = "text";`.
fn constant_at(tokens: &[Token], k: usize) -> Option<(String, String, usize)> {
    let at = |n: usize| tokens.get(k + n).map(|t| &t.tok);
    let Some(Tok::Ident(name)) = at(1) else { return None };
    if at(2) != Some(&Tok::Punct(':')) || at(3) != Some(&Tok::Punct('&')) {
        return None;
    }
    // (`&'static str` has its lifetime between the two.)
    let n = if at(4) == Some(&Tok::Other) { 5 } else { 4 };
    if at(n) != Some(&Tok::Ident("str".into())) || at(n + 1) != Some(&Tok::Punct('=')) || at(n + 3) != Some(&Tok::Punct(';')) {
        return None;
    }
    let Some(Tok::Str(text)) = at(n + 2) else { return None };
    Some((name.clone(), text.clone(), tokens.get(k + 1)?.line))
}

// ---------------------------------------------------------------------------------------------
// Reading the workspace

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Rust files of the workspace's crates and apps, in a fixed order.
fn source_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() && depth < 16 {
                walk(&path, depth + 1, files);
            } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for group in ["crates", "apps"] {
        let Ok(members) = std::fs::read_dir(root.join(group)) else { continue };
        let mut sources: Vec<PathBuf> = members.filter_map(Result::ok).map(|member| member.path().join("src")).collect();
        sources.sort();
        for source in sources {
            walk(&source, 0, &mut files);
        }
    }
    files
}

/// What the scan found.
#[derive(Debug, Default)]
struct Collected {
    found: Vec<Found>,
    /// Constants that `tl!` names and that no source defines as a `&str`.
    unresolved: Vec<String>,
    /// How many times `tl!` was given text, whichever string it was.
    texts: usize,
    /// Every string literal in the workspace's sources.
    literals: HashSet<String>,
}

fn collect(root: &Path) -> Collected {
    let mut scans: Vec<(String, Scan)> = Vec::new();
    for path in source_files(root) {
        let Ok(source) = std::fs::read_to_string(&path) else { panic!("{} can't be read", path.display()) };
        let shown = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        scans.push((shown, scan(&source)));
    }
    let mut collected = Collected::default();
    let mut found: Vec<Found> = Vec::new();
    for (file, scanned) in &scans {
        for lookup in &scanned.lookups {
            match lookup {
                Lookup::Text(text, line) => found.push(Found { english: text.clone(), origin: format!("{file}:{line}") }),
                Lookup::Constant(name, _) => {
                    let defined: Vec<Found> = scans
                        .iter()
                        .flat_map(|(file, scanned)| scanned.constants.iter().map(move |(n, text, line)| (file, n, text, line)))
                        .filter(|(_, n, ..)| *n == name)
                        .map(|(file, _, text, line)| Found { english: text.clone(), origin: format!("{file}:{line}") })
                        .collect();
                    if defined.is_empty() {
                        collected.unresolved.push(name.clone());
                    }
                    found.extend(defined);
                }
            }
        }
        collected.texts += scanned.lookups.iter().filter(|l| matches!(l, Lookup::Text(..))).count();
        collected.literals.extend(scanned.strings.iter().cloned());
    }
    // What the code cuts out of its strings: the abbreviation of a month is the first three letters of its name.
    collected.literals.extend((1..=12).map(|month| mailcraft_time::month_short(month).to_string()));
    // The commands: the label of each, and each part of its location.
    for spec in mailcraft_engine::cmd::registry().all() {
        found.push(Found { english: spec.label.to_string(), origin: spec.id.to_string() });
        found.extend(spec.location.split(LOCATION_SEPARATOR).map(|part| Found { english: part.to_string(), origin: spec.id.to_string() }));
    }
    for (id, label, location, _) in crate::UI_COMMANDS.iter().filter(|command| SEARCHED_WINDOW_COMMANDS.contains(&command.0)) {
        found.push(Found { english: (*label).to_string(), origin: (*id).to_string() });
        found.extend(location.split(LOCATION_SEPARATOR).map(|part| Found { english: part.to_string(), origin: (*id).to_string() }));
    }
    // Each string once, where it was first written.
    let mut seen = HashSet::new();
    collected.found = found.into_iter().filter(|f| !f.english.is_empty() && seen.insert(f.english.clone())).collect();
    collected.unresolved.sort();
    collected.unresolved.dedup();
    collected
}

// ---------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn lookups(source: &str) -> Vec<Lookup> {
        scan(source).lookups
    }

    fn text(english: &str, line: usize) -> Lookup {
        Lookup::Text(english.into(), line)
    }

    #[test]
    fn the_scan_reads_rust_the_way_the_compiler_does() {
        let source = r####"//! Docs that mention tl!("a doc comment") make no call.
/// Nor does tl!("this one").
macro_rules! tl { ($s:expr) => { $crate::i18n::t($s) }; }
const PLAIN: &str = "Plain text";
pub(crate) static WITH_LIFETIME: &'static str = "Static text";
const CONTINUED: &str = "Line one \
                         continues";
const NUMBER: usize = 3;
const LIST: [&str; 1] = ["not a text constant"];
fn f<'a>(x: &'a str) {
    let a = tl!("Simple");
    let b = tl!(
        "Wrapped"
    );
    let c = tl!("Say \"hi\"\n\tand \\ more \u{2026} \x41 \' end");
    let d = tl!(PLAIN);
    let e = tl!(crate::sample::WITH_LIFETIME);
    let f = tl!(label);
    let g = tl!(maybe.unwrap_or("Fallback text"));
    let h = tl!(r"Raw \n text");
    let i = tl!(r#"Raw "quoted" text"#);
    let j = "a string with tl!(\"inside\") in it makes no call";
    let k = '"'; let l = '\''; let m = '\\'; let n: &'static str = "x";
    /* a block /* nested */ comment with tl!("no") */
    let o = tl!("After the characters");
    let p = tl!{"Braces"};
    let q = (b"bytes", b'x', b'"');
    let r = tl!("");
    let s = tl!(CONTINUED);
    let t = tl!(if x.is_empty() { "One" } else { "Other" });
    let u = format!("{} {}", tl!("First"), tl!("Second"));
    let v = my_tl!("not ours") + tl_not!("nor this") + not_tl("this");
}
"####;
        assert_eq!(
            lookups(source),
            [
                text("Simple", 11),
                text("Wrapped", 13),
                text("Say \"hi\"\n\tand \\ more … A ' end", 15),
                Lookup::Constant("PLAIN".into(), 16),
                Lookup::Constant("WITH_LIFETIME".into(), 17),
                text("Fallback text", 19),
                text("Raw \\n text", 20),
                text("Raw \"quoted\" text", 21),
                text("After the characters", 25),
                text("Braces", 26),
                Lookup::Constant("CONTINUED".into(), 29),
                text("One", 30),
                text("Other", 30),
                text("First", 31),
                text("Second", 31),
            ]
        );
        let constants: Vec<(String, String, usize)> = scan(source).constants;
        assert_eq!(
            constants,
            [
                ("PLAIN".into(), "Plain text".into(), 4),
                ("WITH_LIFETIME".into(), "Static text".into(), 5),
                ("CONTINUED".into(), "Line one continues".into(), 6)
            ]
        );
    }

    #[test]
    fn the_scan_reads_run_word_like_tl() {
        let source = r####"const ROWS: [&str; 2] = [run_word!("Couldn't reach the server: {why}"), run_word!(
    "network: {text}"
)];
let a = run_word!["Brackets"];
let b = my_run_word!("not ours") + run_words!("nor this") + run_word("this is a function");
// run_word!("a comment")
let c = "run_word!(\"inside a string\")";
"####;
        assert_eq!(
            lookups(source),
            [text("Couldn't reach the server: {why}", 1), text("network: {text}", 2), text("Brackets", 4)],
            "a row of run_words is a lookup, as a `tl!` is, so every catalog must have it"
        );
    }

    #[test]
    fn the_literals_of_a_source_are_those_it_can_show() {
        let source = r####"#[error("network: {0}")]
fn f() {
    log::warn!("stays in the log: {e}", e = "and this");
    let said = format!("Couldn't reach the server: {why}");
    log::debug!(
        "over two lines"
    );
}
#[cfg(test)]
mod tests {
    fn g() { let _ = "a test's own words"; }
}
#[cfg(test)]
#[path = "x_tests.rs"]
mod named;
const AFTER: &str = "after the tests";
"####;
        let seen: Vec<(String, usize, bool)> = literals_of(source).into_iter().map(|l| (l.text, l.line, l.logged)).collect();
        assert_eq!(
            seen,
            [
                ("network: {0}".to_string(), 1, false),
                ("stays in the log: {e}".to_string(), 3, true),
                ("and this".to_string(), 3, true),
                ("Couldn't reach the server: {why}".to_string(), 4, false),
                ("over two lines".to_string(), 6, true),
                ("x_tests.rs".to_string(), 14, false),
                ("after the tests".to_string(), 16, false),
            ]
        );
        for junk in ["", "#[cfg(test)]", "#[cfg(test)] mod tests {", "log::warn!(", "log::", "\"unfinished"] {
            let _ = literals_of(junk);
        }
    }

    #[test]
    fn the_scan_never_stops_at_source_it_does_not_understand() {
        for source in [
            "",
            "tl!",
            "tl!(",
            "tl!(\"unfinished",
            "tl!(\"a\"",
            "r#\"unfinished",
            "'",
            "'\\",
            "\"\\",
            "/*",
            "const X: &str =",
            "tl!(((",
            "tl!)",
            "\u{feff}tl!(\"a\")",
        ] {
            let _ = scan(source);
        }
        assert_eq!(lookups("\u{feff}tl!(\"a\")"), [text("a", 1)]);
        assert!(lookups("tl!(\"unfinished").is_empty());
    }

    #[test]
    fn the_strings_of_this_workspace_are_found() {
        let found = interface_strings();
        let has = |english: &str| found.iter().any(|f| f.english == english);
        let where_is = |english: &str| found.iter().find(|f| f.english == english).map(|f| f.origin.clone()).unwrap_or_default();
        assert!(found.len() > 1000, "{} strings", found.len());
        assert!(collected().texts > 1000, "{} calls", collected().texts);
        // A call across several lines, text with a quote in it, and a constant's.
        assert!(has("Say yes only if you asked the agent for this. Text in a message can tell an agent to do things you didn't ask for."));
        assert!(has("\"{name}\" will be deleted for good."));
        assert!(has("Turned off by your organisation"));
        // Every command's label and every part of its location.
        for spec in mailcraft_engine::cmd::registry().all() {
            assert!(has(spec.label), "{}: {}", spec.id, spec.label);
            for part in spec.location.split(LOCATION_SEPARATOR).filter(|part| !part.is_empty()) {
                assert!(has(part), "{}: {part}", spec.id);
            }
        }
        // A string is listed where it is first written: in the sources, before the commands.
        assert!(where_is("Options").contains(".rs:"), "{}", where_is("Options"));
        // The window commands that command search lists are all there.
        for id in SEARCHED_WINDOW_COMMANDS {
            let command = crate::UI_COMMANDS.iter().find(|c| c.0 == id).unwrap_or_else(|| panic!("{id} is not a window command any more"));
            assert!(has(command.1), "{id}");
        }
    }

    #[test]
    fn each_string_is_listed_once_with_the_place_it_is_first_written() {
        let found = interface_strings();
        let mut seen = HashSet::new();
        for f in found {
            assert!(seen.insert(f.english.as_str()), "{:?} twice", f.english);
            assert!(!f.english.is_empty() && !f.origin.is_empty());
            let in_sources = f.origin.contains(".rs:");
            let in_commands = !in_sources && !f.origin.contains('/') && f.origin.contains('.');
            assert!(in_sources || in_commands, "{}: {:?}", f.origin, f.english);
        }
        assert!(found.iter().any(|f| !f.origin.contains(".rs:")), "some strings are only a command's label or location");
    }

    #[test]
    fn every_catalog_parses_without_a_bad_line() {
        for row in LANGUAGES {
            let parsed = catalog::parse(row.source);
            let errors: Vec<String> = parsed.errors.iter().map(ToString::to_string).collect();
            assert!(errors.is_empty(), "{}.tsv: {errors:#?}", row.code);
        }
    }

    #[test]
    fn the_placeholders_of_every_entry_are_the_ones_of_its_english() {
        for row in LANGUAGES {
            for entry in catalog::parse(row.source).entries {
                assert_eq!(
                    catalog::placeholders(&entry.english),
                    catalog::placeholders(&entry.translation),
                    "{}.tsv line {}: {:?}",
                    row.code,
                    entry.line,
                    entry.english
                );
            }
        }
    }

    #[test]
    fn no_catalog_entry_is_stale() {
        for row in LANGUAGES {
            let stale = stale_entries(row.source);
            assert!(stale.is_empty(), "{}.tsv holds {} strings that the code doesn't have (renamed or removed?): {stale:#?}", row.code, stale.len());
        }
    }

    /// Every catalog stays whole as the interface grows: a string the interface draws and a
    /// catalog lacks fails here, with the first ones it lacks. Run it on its own to see them:
    /// `cargo test -p mailcraft-ui-egui every_interface_string`.
    #[test]
    fn every_interface_string_has_an_entry_in_every_catalog() {
        for row in LANGUAGES {
            let missing = missing_entries(row.source);
            assert!(
                missing.is_empty(),
                "{}.tsv lacks {} strings, such as {:#?}",
                row.code,
                missing.len(),
                missing.iter().take(12).collect::<Vec<_>>()
            );
        }
    }

    /// The names of the months and of the days of the week reach `tl!` from `mailcraft_time`, which the scan can't
    /// follow: the calendar's titles and columns draw every one of them, full and short, and every catalog has them.
    #[test]
    fn every_month_and_weekday_name_the_calendar_draws_has_an_entry_in_every_catalog() {
        let months = (1..=12).flat_map(|month| [mailcraft_time::month_name(month), mailcraft_time::month_short(month)]);
        let days = (0..7).flat_map(|n| {
            let day = mailcraft_time::Weekday::from_index(n);
            [day.name(), day.short()]
        });
        let names: Vec<&str> = months.chain(days).collect();
        assert_eq!(names.len(), 38);
        for row in LANGUAGES {
            let has: HashSet<String> = catalog::parse(row.source).entries.into_iter().map(|entry| entry.english).collect();
            let missing: Vec<&str> = names.iter().copied().filter(|name| !has.contains(*name)).collect();
            assert!(missing.is_empty(), "{}.tsv lacks {missing:?}", row.code);
        }
    }

    #[test]
    fn the_checks_of_a_catalog_find_what_they_are_for() {
        let is_looked_up = |s: &str| interface_strings().iter().any(|f| f.english == s);
        let looked_up =
            interface_strings().iter().find(|f| is_text(&f.english) && !f.english.contains(['\n', '\t', '\\'])).map(|f| f.english.clone());
        // A string the code has, and that `tl!` isn't given: a table's.
        let table = collected().literals.iter().filter(|l| is_text(l) && !is_looked_up(l) && !l.contains(['\n', '\t', '\\', '{'])).min().cloned();
        let (Some(looked_up), Some(table)) = (looked_up, table) else { panic!("the workspace has strings of both kinds") };
        let source = format!(
            "# header\n\t{looked_up}\tune traduction\n\t{table}\tune autre\n\tA string nobody wrote \u{2603}\tpersonne\n\n\t{looked_up} \tdeuxi\u{e8}me\n"
        );
        assert_eq!(
            stale_entries(&source),
            ["line 4: \"A string nobody wrote \u{2603}\"".to_string(), format!("line 6: {:?}", format!("{looked_up} "))]
        );
        // The strings a catalog lacks are all those of the interface but the ones it has.
        let everything = missing_entries("");
        assert!(everything.len() > 1000 && everything.iter().all(|s| is_text(s)));
        let lacking = missing_entries(&source);
        assert_eq!(lacking.len(), everything.len() - 1);
        assert!(everything.contains(&looked_up.as_str()) && !lacking.contains(&looked_up.as_str()));
        // A string under a context is not the plain string the interface asks for.
        let in_context = format!("a context\t{looked_up}\tune traduction\n");
        assert_eq!(missing_entries(&in_context).len(), everything.len());
        assert!(stale_entries(&in_context).is_empty());
        // The abbreviation of a month is cut from its name: written nowhere, and the interface's all the same.
        assert!(stale_entries("\tJan\tjanv.\n\tOct\toct.\n").is_empty());
    }

    /// Prints the strings that translators need, one JSON object a line behind the word
    /// `interface-string`: `{"en", "where", "context"}`. Run it on its own, with the output shown:
    /// `cargo test -p mailcraft-ui-egui print_interface_strings -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints the strings for translators"]
    fn print_interface_strings() {
        for f in interface_strings().iter().filter(|f| is_text(&f.english)) {
            println!("interface-string {}", serde_json::json!({"en": f.english, "where": f.origin, "context": ""}));
        }
    }
}
