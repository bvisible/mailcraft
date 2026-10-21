// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what the census of the sentences `run_words` reads back needs of the
// sources (their string literals) and of a template (its placeholders), kept from the catalogue code this fork had before it
// took upstream's (`mailcraft-i18n`).
//! The string literals of a source file ([`literals_of`]), read by a small lexer, and the placeholders of a template
//! ([`placeholders`]): for the tests of `run_words`, which hold the crates' sentences to its rows.

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

/// The `{name}` placeholders of a template, sorted (a name that comes twice is listed twice). A
/// name is letters, digits and underscores; any other braces are text.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    // Each placeholder is the text between a `}` and the last `{` before it.
    while let Some((head, tail)) = rest.split_once('}') {
        if let Some((_, name)) = head.rsplit_once('{')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            names.push(name);
        }
        rest = tail;
    }
    names.sort_unstable();
    names
}
