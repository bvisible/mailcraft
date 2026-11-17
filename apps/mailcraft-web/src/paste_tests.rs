// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): tests of paste.rs (neoffice-maintenance#1705).
//! What a paste in the browser carries as HTML, handed once while fresh, without the pictures written into it.

use super::*;

#[test]
fn the_html_of_a_paste_is_handed_once_to_the_paste_made_from_it() {
    let mut caught = Caught::default();
    caught.caught(Some("<b>Bold</b>"), 1_000.0);
    assert_eq!(caught.take(1_050.0).as_deref(), Some("<b>Bold</b>"));
    // The next paste of the body asks again: nothing, never the same HTML twice.
    assert_eq!(caught.take(1_060.0), None);
}

#[test]
fn html_that_waited_too_long_is_another_pastes_and_is_not_handed() {
    let mut caught = Caught::default();
    caught.caught(Some("<b>Bold</b>"), 1_000.0);
    assert_eq!(caught.take(1_000.0 + FRESH_MS + 1.0), None);
    // A clock that went back is no answer either.
    caught.caught(Some("<b>Bold</b>"), 5_000.0);
    assert_eq!(caught.take(4_000.0), None);
}

#[test]
fn a_paste_without_html_forgets_the_html_of_the_one_before() {
    let mut caught = Caught::default();
    caught.caught(Some("<b>Bold</b>"), 1_000.0);
    caught.caught(None, 1_100.0);
    assert_eq!(caught.take(1_150.0), None);
    caught.caught(Some("   "), 1_200.0);
    assert_eq!(caught.take(1_250.0), None);
}

#[test]
fn the_pictures_written_into_a_pasted_html_are_left_out_and_the_rest_kept() {
    let picture = format!("data:image/png;base64,{}", "A".repeat(64));
    let html = format!(
        "<p><b>Offer</b> <a href=\"https://example.org/a\">terms</a></p><img src=\"{picture}\" alt=\"logo\"><img src='{picture}'><div style=\"background:url({picture})\">x</div><table><tr><td>1</td></tr></table>"
    );
    let out = without_inline_pictures(&html);
    assert_eq!(
        out,
        "<p><b>Offer</b> <a href=\"https://example.org/a\">terms</a></p><img src=\"\" alt=\"logo\"><img src=''><div style=\"background:url()\">x</div><table><tr><td>1</td></tr></table>"
    );
    // Words that say « data: » in the text, and an address never closed, are left as they are.
    assert_eq!(without_inline_pictures("<p>data: 12 rows</p>"), "<p>data: 12 rows</p>");
    assert_eq!(without_inline_pictures("<img src=\"data:image/png;base64,AAA"), "<img src=\"data:image/png;base64,AAA");
    // Upper case is an address too, and what is around keeps its own case.
    assert_eq!(without_inline_pictures("<IMG SRC=\"DATA:image/gif;base64,R0\">Ünïcode"), "<IMG SRC=\"\">Ünïcode");
}

#[test]
fn a_document_whose_pictures_weigh_more_than_a_paste_takes_still_brings_its_formatting() {
    let mut caught = Caught::default();
    let heavy = format!("<p><b>Report</b></p><img src=\"data:image/png;base64,{}\">", "A".repeat(MAX_HTML + 1));
    caught.caught(Some(&heavy), 1_000.0);
    assert_eq!(caught.take(1_010.0).as_deref(), Some("<p><b>Report</b></p><img src=\"\">"));
    // Over the limit without them: the text alone, as a paste without HTML.
    let long = format!("<p>{}</p>", "x".repeat(MAX_HTML));
    caught.caught(Some(&long), 2_000.0);
    assert_eq!(caught.take(2_010.0), None);
}
