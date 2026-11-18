// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): the tests of a category kept as a JMAP keyword (category_keywords.rs).

use super::*;

/// RFC 8621 §4.1.1 and IMAP's atom (what frappe_webmail's façade holds a keyword to): printable ASCII but `( ) { ] % * " \`
/// and the space, 1 to 255 of them.
fn a_keyword(k: &str) -> bool {
    (1..=255).contains(&k.len()) && k.bytes().all(|b| (0x21..=0x7e).contains(&b) && !b"(){]%*\"\\".contains(&b))
}

#[test]
fn a_category_is_a_keyword_of_its_own_and_comes_back_as_it_was_written() {
    for (name, keyword) in [
        ("Rouge", "$cat:=52ouge"),
        ("rouge", "$cat:rouge"),
        ("Fournisseur à relancer", "$cat:=46ournisseur=20=c3=a0=20relancer"),
        ("Clients (VIP) 100%", "$cat:=43lients=20=28=56=49=50=29=20100=25"),
        ("a/b~c\\d\"e", "$cat:a=2fb=7ec=5cd=22e"),
        ("日本", "$cat:=e6=97=a5=e6=9c=ac"),
    ] {
        assert_eq!(category_keyword(name).as_deref(), Some(keyword), "{name}");
        assert!(a_keyword(keyword), "{keyword}");
        assert_eq!(keyword_category(keyword).as_deref(), Some(name), "{keyword}");
        // A server keeps it in lower case, or in capitals: the same name.
        assert_eq!(keyword_category(&keyword.to_lowercase()).as_deref(), Some(name));
        assert_eq!(keyword_category(&keyword.to_uppercase()).as_deref(), Some(name));
    }
    // Trimmed as a category is; none for a name that is none.
    assert_eq!(category_keyword("  Rouge \t").as_deref(), Some("$cat:=52ouge"));
    assert_eq!(category_keyword("   "), None);
    // Too long for a keyword: not kept on the server.
    // (« é » is two bytes, `=c3=a9`: 41 of them make a keyword of 251, 42 one of 257.)
    assert!(category_keyword(&"é".repeat(41)).is_some_and(|k| k.len() == 251));
    assert!(category_keyword(&"é".repeat(42)).is_none());
    // The longest name (`MAX_NAME`) of small letters is one.
    assert!(category_keyword(&"a".repeat(crate::MAX_NAME + 10)).is_some_and(|k| k.len() == PREFIX.len() + crate::MAX_NAME));
}

#[test]
fn a_keyword_that_isnt_a_category_is_none() {
    for keyword in [
        "$seen",
        "$flagged",
        "$forwarded",
        "$cat:",
        "$cat",
        "cat:rouge",
        "$cat:=",
        "$cat:=4",
        "$cat:=zz",
        "$cat:a b",
        "$cat:=ff=fe",
        "$cat:=0arouge",
        "$cat:=20rouge",
        "$cat:rouge=20",
        "$cat:=c3",
    ] {
        assert_eq!(keyword_category(keyword), None, "{keyword}");
    }
    assert_eq!(keyword_category(&format!("$cat:{}", "a".repeat(300))), None);
}

#[test]
fn a_messages_categories_are_its_category_keywords_each_once() {
    let keywords = ["$seen", "$cat:=52ouge", "$cat:=72ouge", "$cat:rouge", "work", "$cat:=56ert", "$cat:=zz"];
    assert_eq!(categories_of(keywords), ["Rouge", "Vert"]);
    let many: Vec<String> = (0..200).map(|i| format!("$cat:c{i}")).collect();
    assert_eq!(categories_of(many.iter().map(String::as_str)).len(), MAX_CATEGORIES);
    assert_eq!(kept_categories(&["Rouge".into(), "é".repeat(60), " ".into()]), ["Rouge"]);
}

proptest::proptest! {
    #[test]
    fn any_name_is_a_keyword_that_gives_it_back_or_none(name in "\\PC{0,80}") {
        if let Some(keyword) = category_keyword(&name) {
            proptest::prop_assert!(a_keyword(&keyword), "{}", keyword);
            proptest::prop_assert_eq!(keyword_category(&keyword), crate::clean_name(&name));
        }
    }

    #[test]
    fn any_keyword_reads_without_trouble(keyword in "\\PC{0,300}") {
        let _ = keyword_category(&keyword);
    }
}
