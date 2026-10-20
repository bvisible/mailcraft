// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Language catalogs: what the interface shows in a language other than English.
//!
//! The interface's strings are English and are the keys ([`crate::i18n::t`]). A catalog is a text
//! file with one line for each string that has a translation, in three TAB-separated columns:
//!
//! ```text
//! context<TAB>English<TAB>translation
//! ```
//!
//! - `context` is empty for nearly every line. It tells apart English words that need two
//!   translations (`folder` for the Archive folder as against the Archive command); a string
//!   with a context is looked up with [`crate::i18n::tc`], and a plain one without.
//! - `\n`, `\t` and `\\` write a new line, a TAB and a backslash. Nothing else is escaped.
//! - A line that starts with `#` is a comment, and a blank line is nothing. Columns are taken as
//!   they are written: spaces at their ends are part of the text.
//! - The `{name}` placeholders of the English are in the translation too, once for each time the
//!   English has them, and no other braces make a placeholder.
//! - A string without a line shows in English, so a catalog can start empty and grow.
//!
//! The text is compiled in and parsed the first time a language is used. A line that doesn't
//! parse is skipped and logged, never fatal: the rest of the catalog stays good. The tests refuse
//! any such line, and any entry that isn't a string of the interface.
//!
//! # Adding a language
//! One row in [`LANGUAGES`] and the file that row names, beside this one. The code is the
//! language's BCP 47 tag in lower case (`de`, `pt-br`, `zh-hans`): it is the value of the
//! `language` setting, and a locale tag with more subtags (`pt-BR`, `de_CH.UTF-8`) finds the row
//! whose code is the longest of its leading subtags.

use std::collections::HashMap;
use std::collections::hash_map::Entry as Slot;
use std::fmt;
use std::sync::OnceLock;

/// How many lines of errors a catalog logs when it loads; the tests show them all.
const LOGGED_ERRORS: usize = 10;

/// A language that has a catalog.
#[derive(Clone, Copy, Debug)]
pub struct LangInfo {
    /// The language's tag in lower case (`fr`, `pt-br`): also the value of the `language` setting.
    pub code: &'static str,
    /// The language's own name, as the list of languages shows it.
    pub name: &'static str,
    /// The text of the catalog.
    pub source: &'static str,
}

/// Every language that has a catalog. Adding one is a row here and the file it names.
pub static LANGUAGES: &[LangInfo] = &[LangInfo { code: "fr", name: "Français", source: include_str!("fr.tsv") }];

/// A language of [`LANGUAGES`], by its row. Only [`Id::new`] and [`find`] make one, so it always
/// names a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(u8);

impl Id {
    /// The language of row `index`, if there is one.
    pub fn new(index: usize) -> Option<Id> {
        if index < LANGUAGES.len() { u8::try_from(index).ok().map(Id) } else { None }
    }

    /// Every language that has a catalog, in the table's order.
    pub fn all() -> impl Iterator<Item = Id> {
        (0..LANGUAGES.len()).filter_map(Id::new)
    }

    /// The row's position in [`LANGUAGES`].
    pub fn index(self) -> u8 {
        self.0
    }

    /// The row.
    pub fn row(self) -> Option<&'static LangInfo> {
        LANGUAGES.get(usize::from(self.0))
    }
}

/// The language of a locale tag (`fr`, `fr-CH`, `FR_ch.UTF-8@euro`): the row whose code is the
/// longest of the tag's leading subtags, compared without regard to case or to `-` against `_`.
/// A tag that no row matches (or that isn't a tag at all) has no catalog.
pub fn find(tag: &str) -> Option<Id> {
    // (A POSIX locale name's encoding and modifier say nothing of the language.)
    let tag = tag.trim();
    let tag = tag.split(['.', '@']).next().unwrap_or(tag);
    let mut end = tag.len();
    loop {
        let candidate = tag.get(..end)?;
        if let Some(index) = LANGUAGES.iter().position(|row| same_tag(row.code, candidate)) {
            return Id::new(index);
        }
        // Without its last subtag.
        end = candidate.rfind(['-', '_'])?;
    }
}

/// Whether two language tags are the same: letters of any case, and `-` or `_` between subtags.
fn same_tag(a: &str, b: &str) -> bool {
    let normal = |byte: u8| if byte == b'_' { b'-' } else { byte.to_ascii_lowercase() };
    a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| normal(x) == normal(y))
}

/// `english` in the language `id`, when its catalog has it (under `context`, which is empty for
/// the plain form).
pub fn translate(id: Id, context: &str, english: &str) -> Option<&'static str> {
    catalog(id)?.get(context, english)
}

/// The loaded catalog of a language, parsed the first time it is wanted.
fn catalog(id: Id) -> Option<&'static Catalog> {
    static CATALOGS: OnceLock<Vec<OnceLock<Catalog>>> = OnceLock::new();
    let row = id.row()?;
    let cells = CATALOGS.get_or_init(|| LANGUAGES.iter().map(|_| OnceLock::new()).collect());
    Some(cells.get(usize::from(id.index()))?.get_or_init(|| Catalog::new(parse_logged(row))))
}

/// A parsed catalog: the strings with no context, and for each context its own.
#[derive(Default)]
struct Catalog {
    plain: HashMap<String, String>,
    with_context: HashMap<String, HashMap<String, String>>,
}

impl Catalog {
    fn new(entries: Vec<Entry>) -> Catalog {
        let mut catalog = Catalog::default();
        for Entry { context, english, translation, .. } in entries {
            let strings = if context.is_empty() { &mut catalog.plain } else { catalog.with_context.entry(context).or_default() };
            strings.insert(english, translation);
        }
        catalog
    }

    fn get(&self, context: &str, english: &str) -> Option<&str> {
        let strings = if context.is_empty() { &self.plain } else { self.with_context.get(context)? };
        strings.get(english).map(String::as_str)
    }
}

/// The entries of a language's catalog, with what is wrong with the rest said in the log.
fn parse_logged(row: &LangInfo) -> Vec<Entry> {
    let parsed = parse(row.source);
    for error in parsed.errors.iter().take(LOGGED_ERRORS) {
        log::warn!("{} catalog: {error} (the line is skipped: its text shows in English)", row.code);
    }
    if parsed.errors.len() > LOGGED_ERRORS {
        log::warn!("{} catalog: and {} more lines that don't parse", row.code, parsed.errors.len() - LOGGED_ERRORS);
    }
    parsed.entries
}

/// One line of a catalog that parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its line in the file, from 1.
    pub line: usize,
    pub context: String,
    pub english: String,
    pub translation: String,
}

/// A line of a catalog that doesn't parse, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Its line in the file, from 1.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// What a catalog's text came to: the lines that parsed, and for each of the others why not.
#[derive(Debug, Default)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    pub errors: Vec<ParseError>,
}

/// Read a catalog. Every line that doesn't parse is reported, whichever comes first; the others
/// are entries. A string that has been seen under a context is refused the second time under it.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut first_seen: HashMap<(String, String), usize> = HashMap::new();
    // (A byte order mark at the start is an editor's doing.)
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut entry = match parse_line(line) {
            Ok(entry) => entry,
            Err(message) => {
                parsed.errors.push(ParseError { line: number, message });
                continue;
            }
        };
        entry.line = number;
        match first_seen.entry((entry.context.clone(), entry.english.clone())) {
            Slot::Occupied(first) => {
                let message = format!("{:?} is there already, on line {}", entry.english, first.get());
                parsed.errors.push(ParseError { line: number, message });
            }
            Slot::Vacant(slot) => {
                slot.insert(number);
                parsed.entries.push(entry);
            }
        }
    }
    parsed
}

/// One line that isn't blank or a comment (its `line` is the caller's to set).
fn parse_line(line: &str) -> Result<Entry, String> {
    let mut columns = line.split('\t');
    let (Some(context), Some(english), Some(translation), None) = (columns.next(), columns.next(), columns.next(), columns.next()) else {
        return Err(format!("expected 3 TAB-separated columns (context, English, translation), found {}", line.split('\t').count()));
    };
    if let Some(bad) = [context, english, translation].into_iter().find_map(|column| column.chars().find(|c| c.is_control())) {
        return Err(format!("a control character (U+{:04X}); a new line or a TAB is written \\n or \\t", u32::from(bad)));
    }
    let english = unescape(english).map_err(|e| format!("in the English: {e}"))?;
    let translation = unescape(translation).map_err(|e| format!("in the translation: {e}"))?;
    if english.is_empty() {
        return Err("the English is empty".into());
    }
    if translation.is_empty() {
        return Err(format!("the translation of {english:?} is empty (a string without a line shows in English)"));
    }
    let (wanted, found) = (placeholders(&english), placeholders(&translation));
    if wanted != found {
        let list = |names: &[&str]| {
            if names.is_empty() { "none".to_string() } else { names.iter().map(|name| format!("{{{name}}}")).collect::<Vec<_>>().join(" ") }
        };
        return Err(format!("the translation of {english:?} has the placeholders {} where the English has {}", list(&found), list(&wanted)));
    }
    Ok(Entry { line: 0, context: context.to_string(), english, translation })
}

/// Text as a catalog's column holds it: a backslash followed by `n`, `t` or `\` is a new line, a
/// TAB or a backslash.
pub fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => return Err(format!("unknown escape \\{other} (only \\n, \\t and \\\\ are)")),
            None => return Err("a backslash at the end (write \\\\ for one)".into()),
        }
    }
    Ok(out)
}

/// The text as a catalog's column holds it ([`unescape`]'s inverse).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The entries of a catalog text, loaded as a language's would be.
    fn loaded(text: &str) -> Catalog {
        let parsed = parse(text);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        Catalog::new(parsed.entries)
    }

    #[test]
    fn a_catalog_reads_entries_comments_and_blank_lines() {
        let parsed = parse("# a comment\n\n\tReply\tRépondre\nfolder\tArchive\tArchives\n   \n#\tnot\tan entry\n\tArchive\tArchiver\n");
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let seen: Vec<(usize, &str, &str, &str)> =
            parsed.entries.iter().map(|e| (e.line, e.context.as_str(), e.english.as_str(), e.translation.as_str())).collect();
        assert_eq!(seen, [(3, "", "Reply", "Répondre"), (4, "folder", "Archive", "Archives"), (7, "", "Archive", "Archiver")]);
        // Windows line ends and a byte order mark are an editor's doing.
        let parsed = parse("\u{feff}# header\r\n\tReply\tRépondre\r\n");
        assert!(parsed.errors.is_empty() && parsed.entries.len() == 1, "{parsed:?}");
        assert_eq!(parsed.entries.first().map(|e| (e.line, e.translation.as_str())), Some((2, "Répondre")));
        // Nothing at all is a catalog without entries.
        assert!(parse("").entries.is_empty() && parse("# only a header\n").errors.is_empty());
    }

    #[test]
    fn columns_are_taken_as_written() {
        let text = "\t  Loading folders…\t  Chargement des dossiers…\n\tTwo\\nlines\\tand a tab \\\\ and a backslash\tDeux\\nlignes\\tet \\\\\n";
        let parsed = parse(text);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let english: Vec<&str> = parsed.entries.iter().map(|e| e.english.as_str()).collect();
        assert_eq!(english, ["  Loading folders…", "Two\nlines\tand a tab \\ and a backslash"]);
        assert_eq!(parsed.entries.first().map(|e| e.translation.as_str()), Some("  Chargement des dossiers…"));
        assert_eq!(parsed.entries.last().map(|e| e.translation.as_str()), Some("Deux\nlignes\tet \\"));
    }

    #[test]
    fn escapes_go_both_ways_and_only_three_exist() {
        for text in ["plain", "a\nb", "a\tb", "back\\slash", "\\n is not a new line here", "…", "\n\t\\"] {
            assert_eq!(unescape(&escape(text)).as_deref(), Ok(text), "{text:?}");
        }
        assert_eq!(unescape("a\\nb\\tc\\\\d").as_deref(), Ok("a\nb\tc\\d"));
        assert!(unescape("\\x").is_err_and(|e| e.contains("unknown escape \\x")));
        assert!(unescape("\\r").is_err());
        assert!(unescape("ends with a backslash \\").is_err_and(|e| e.contains("backslash at the end")));
        assert_eq!(escape("a\r"), "a\r", "only the three are written");
    }

    #[test]
    fn bad_lines_are_refused_each_with_its_line_and_why() {
        let text = [
            "\tReply\tRépondre",          // 1: fine
            "Reply\tRépondre",            // 2: two columns
            "\tReply\tRépondre\tmore",    // 3: four columns
            "just words",                 // 4: one column
            "\t\tRépondre",               // 5: no English
            "\tReply\t",                  // 6: no translation
            "\tReply\tRép\\ondre",        // 7: unknown escape
            "\tReply\tRépondre\\",        // 8: a backslash at the end
            "\tReply\tRép\u{7}ondre",     // 9: a control character
            "\tOpen {name}\tOuvrir",      // 10: a placeholder lost
            "\tOpen\tOuvrir {name}",      // 11: a placeholder added
            "\tOpen {a} {b}\tOuvrir {a}", // 12: one of two lost
            "\tOpen {a}\tOuvrir {a} {a}", // 13: one too many
            "\tOpen { a }\tOuvrir {b}",   // 14: braces that aren't placeholders: only {b} is one
            "\tForward\tTransférer",      // 15: fine
        ]
        .join("\n");
        let parsed = parse(&text);
        let lines: Vec<usize> = parsed.errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, (2..=14).collect::<Vec<_>>(), "{:#?}", parsed.errors);
        let said = |line: usize| parsed.errors.iter().find(|e| e.line == line).map(ToString::to_string).unwrap_or_default();
        assert!(said(2).starts_with("line 2: expected 3 TAB-separated columns") && said(2).contains("found 2"), "{}", said(2));
        assert!(said(3).contains("found 4") && said(4).contains("found 1"));
        assert!(said(5).contains("English is empty"), "{}", said(5));
        assert!(said(6).contains("translation of \"Reply\" is empty"), "{}", said(6));
        assert!(said(7).contains("in the translation") && said(7).contains("unknown escape \\o"), "{}", said(7));
        assert!(said(8).contains("backslash at the end"), "{}", said(8));
        assert!(said(9).contains("control character (U+0007)"), "{}", said(9));
        assert!(said(10).contains("has the placeholders none where the English has {name}"), "{}", said(10));
        assert!(said(11).contains("has the placeholders {name} where the English has none"), "{}", said(11));
        assert!(said(12).contains("{a} where the English has {a} {b}"), "{}", said(12));
        assert!(said(13).contains("{a} {a} where the English has {a}"), "{}", said(13));
        assert!(said(14).contains("{b} where the English has none"), "{}", said(14));
        // The lines that are fine are all there.
        let kept: Vec<usize> = parsed.entries.iter().map(|e| e.line).collect();
        assert_eq!(kept, [1, 15]);
    }

    #[test]
    fn a_string_that_comes_twice_is_refused_the_second_time() {
        let parsed = parse("\tReply\tRépondre\n\tReply\tRépondre à\nfolder\tReply\tClasseur\nfolder\tReply\tAutre\n\tReply all\tRépondre à tous\n");
        let errors: Vec<(usize, &str)> = parsed.errors.iter().map(|e| (e.line, e.message.as_str())).collect();
        assert_eq!(errors, [(2, "\"Reply\" is there already, on line 1"), (4, "\"Reply\" is there already, on line 3")]);
        let kept: Vec<(usize, &str)> = parsed.entries.iter().map(|e| (e.line, e.translation.as_str())).collect();
        assert_eq!(kept, [(1, "Répondre"), (3, "Classeur"), (5, "Répondre à tous")], "the first stays, and another context is not a copy");
    }

    #[test]
    fn placeholders_are_names_in_braces() {
        assert_eq!(placeholders("{n} of {total} items"), ["n", "total"]);
        assert_eq!(placeholders("{b} {a} {b}"), ["a", "b", "b"]);
        assert_eq!(placeholders("none"), Vec::<&str>::new());
        assert_eq!(placeholders("{ spaced } {} {a-b} {é} {ok_1}"), ["ok_1"]);
        assert_eq!(placeholders("{{n}}"), ["n"]);
        assert_eq!(placeholders("{open"), Vec::<&str>::new());
        assert_eq!(placeholders("close} {x}"), ["x"]);
        assert_eq!(placeholders("}{}{}}{x}"), ["x"]);
        // Hostile text stays cheap and never panics.
        let many = "{".repeat(100_000) + "}";
        assert!(placeholders(&many).is_empty());
    }

    #[test]
    fn a_loaded_catalog_answers_under_the_context_asked_for() {
        let catalog = loaded("\tArchive\tArchiver\nfolder\tArchive\tArchives\n\tReply\tRépondre\n");
        assert_eq!(catalog.get("", "Archive"), Some("Archiver"));
        assert_eq!(catalog.get("folder", "Archive"), Some("Archives"));
        assert_eq!(catalog.get("", "Reply"), Some("Répondre"));
        assert_eq!(catalog.get("folder", "Reply"), None, "a context is not a way to the plain form");
        assert_eq!(catalog.get("other", "Archive"), None);
        assert_eq!(catalog.get("", "Nope"), None);
        assert_eq!(catalog.get("", "folder\u{4}Archive"), None, "nothing in a string can reach another context's");
    }

    #[test]
    fn locale_tags_find_the_language_they_name() {
        let fr = find("fr");
        assert!(fr.is_some());
        for tag in ["fr", "FR", "Fr", "fr-CH", "fr_CH", "fr-ch", "fr_CA.UTF-8", "fr_FR@euro", "fr.UTF-8", "  fr-BE  ", "fr-Latn-CH", "fr-x-private"] {
            assert_eq!(find(tag), fr, "{tag:?}");
        }
        // (A tag that merely starts like a language is not that language.)
        for tag in ["", " ", "f", "fra", "frr", "fr1", "français", "en", "en-GB", "de", "C", "POSIX", "-", "_", "-fr", "x-fr", ".fr", "fr\u{0}"] {
            assert_eq!(find(tag), None, "{tag:?}");
        }
        assert_eq!(find(&format!("fr-{}", "x".repeat(10_000))), fr, "a long tail is only subtags");
        assert_eq!(find(&"-".repeat(10_000)), None);
        assert_eq!(find(&"a-".repeat(10_000)), None);
    }

    #[test]
    fn the_table_is_made_of_sound_rows() {
        let mut seen: Vec<&str> = Vec::new();
        for row in LANGUAGES {
            let code = row.code;
            assert!(
                !code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{code:?} is a tag in lower case"
            );
            assert!(!code.starts_with('-') && !code.ends_with('-') && !code.contains("--"), "{code:?}");
            assert!(!seen.contains(&code), "{code:?} is twice in the table");
            seen.push(code);
            assert!(!["en", "auto", "pseudo"].contains(&code), "{code:?} is not a catalog's to take");
            assert!(!row.name.trim().is_empty(), "{code}: its own name");
            assert_eq!(find(code).and_then(Id::row).map(|r| r.code), Some(code), "{code} finds itself");
        }
        assert!(Id::all().count() == LANGUAGES.len() && Id::new(LANGUAGES.len()).is_none() && Id::new(usize::MAX).is_none());
        for id in Id::all() {
            assert_eq!(Id::new(usize::from(id.index())), Some(id));
        }
    }

    #[test]
    fn a_string_a_catalog_lacks_comes_back_as_none_for_the_english_to_stand() {
        for id in Id::all() {
            assert_eq!(translate(id, "", "A string that no catalog will ever hold ☃"), None);
            assert_eq!(translate(id, "no-such-context", "A string that no catalog will ever hold ☃"), None);
        }
    }
}
