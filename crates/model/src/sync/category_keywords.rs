// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): a message's categories kept on a JMAP server, as keywords of its own
// //// Neoffice — (neoffice-maintenance#2487): a category set in a hosting page's tab is still there at the next visit.

//! A category of a message as a keyword of the message on a JMAP server (RFC 8621 §4.1.1), and back.
//!
//! JMAP has no categories, and a keyword is no name: 1 to 255 characters of printable ASCII, none of `( ) { ] % * " \`,
//! no space, and a server may (an IMAP server does) keep it in lower case. So a category is written in a keyword of its own,
//! [`PREFIX`] and then its name byte by byte (UTF-8): a small letter, a digit, `-`, `_` and `.` as they are, any other byte
//! (a capital, a space, an accent…) as `=` and its two hexadecimal digits in lower case. « Rouge » is `$cat:=52ouge`,
//! « Fournisseur à relancer » `$cat:=46ournisseur=20=c3=a0=20relancer`. What comes back is read the other way, in any case
//! of letters; a keyword that isn't one of these, or that doesn't spell a name, is no category (and is never touched: the
//! changes name only the keywords of the categories that changed).

use super::same_category;

/// What a category's keyword starts with.
pub const PREFIX: &str = "$cat:";
/// The longest keyword (RFC 8621 §4.1.1): a category whose keyword would be longer is not kept on the server.
pub const MAX_KEYWORD: usize = 255;
/// The most categories read of one message.
pub const MAX_CATEGORIES: usize = 64;

/// The keyword that keeps the category `name` on a JMAP server; `None` for a name that is none (empty once trimmed) or whose
/// keyword would be longer than [`MAX_KEYWORD`].
pub fn category_keyword(name: &str) -> Option<String> {
    let name = crate::clean_name(name)?;
    let mut keyword = String::from(PREFIX);
    for byte in name.bytes() {
        if keyword.len() > MAX_KEYWORD {
            return None;
        }
        match byte {
            b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => keyword.push(char::from(byte)),
            _ => {
                keyword.push('=');
                keyword.push(hex(byte >> 4));
                keyword.push(hex(byte & 0x0f));
            }
        }
    }
    (keyword.len() <= MAX_KEYWORD).then_some(keyword)
}

/// The category a keyword keeps (any case of its letters); `None` for any other keyword.
pub fn keyword_category(keyword: &str) -> Option<String> {
    if keyword.len() > MAX_KEYWORD || keyword.len() <= PREFIX.len() {
        return None;
    }
    let (head, rest) = keyword.split_at_checked(PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        match byte {
            b'=' => {
                let high = chars.next().and_then(unhex)?;
                let low = chars.next().and_then(unhex)?;
                bytes.push((high << 4) | low);
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => bytes.push(byte.to_ascii_lowercase()),
            _ => return None,
        }
    }
    let name = String::from_utf8(bytes).ok()?;
    // Only what was written as a name: one that comes back otherwise (a control character, spaces at its ends) is none.
    crate::clean_name(&name).filter(|clean| *clean == name)
}

/// The categories among a message's keywords (JMAP's `keywords` object: each name `true`), each once, at most
/// [`MAX_CATEGORIES`].
pub fn categories_of<'a>(keywords: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for keyword in keywords {
        if found.len() >= MAX_CATEGORIES {
            break;
        }
        if let Some(name) = keyword_category(keyword)
            && !found.iter().any(|c| same_category(c, &name))
        {
            found.push(name);
        }
    }
    found
}

/// Of `list`, the categories a JMAP server can keep (those that have a keyword).
pub fn kept_categories(list: &[String]) -> Vec<String> {
    list.iter().filter(|name| category_keyword(name).is_some()).cloned().collect()
}

fn hex(nibble: u8) -> char {
    char::from(if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 })
}

fn unhex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "category_keywords_tests.rs"]
mod tests;
