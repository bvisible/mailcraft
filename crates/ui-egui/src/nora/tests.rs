// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): what Nora's answers say, and what she is given.
//! The answers of the server read as the person is told them, hostile ones included, and what Nora
//! is given of a message being written and of a message of the sample mailbox. Invented people at
//! `example.org` and the sample mailbox's own.

use mailcraft_engine::Session;
use mailcraft_model::rich::{ListKind, Paragraph, Run, Style};
use mailcraft_model::{Address, RichText};
use serde_json::{Value, json};

use super::answer::{self, Task};
use super::own_text::{self, OwnText};
use super::{MAX_ANSWER, MAX_OWN_TEXT, MAX_REPLY, NoraAnswer};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

fn answer(status: u16, body: Value) -> NoraAnswer {
    NoraAnswer { id: 1, status, body: body.to_string() }
}

/// What `frappe.throw` sends: the sentence the desk shows, in a list written as JSON text of
/// messages that are each JSON text.
fn refusal(said: &str) -> Value {
    let shown = json!({"message": said, "title": "Message", "indicator": "red"}).to_string();
    json!({"exc_type": "ValidationError", "exception": format!("frappe.exceptions.ValidationError: {said}"), "_server_messages": json!([shown]).to_string()})
}

#[test]
fn a_draft_comes_out_of_the_answer() {
    let ok = answer(200, json!({"message": {"success": true, "draft_text": "  Bonjour Ada,\n\nC'est noté.  ", "draft_html": "<p>…</p>"}}));
    assert_eq!(answer::draft(&ok), Ok("Bonjour Ada,\n\nC'est noté.".to_string()));
}

#[test]
fn a_refusal_says_what_the_server_said() {
    let refused = answer(417, refusal("Nora refused to process this message: it contains <b>instructions</b> &amp; more."));
    assert_eq!(
        answer::draft(&refused),
        Err("Nora couldn't write the reply: Nora refused to process this message: it contains instructions & more.".to_string())
    );
    let crashed = answer(500, json!({"exc_type": "KeyError", "exception": "frappe.exceptions.ValidationError: Access denied"}));
    assert_eq!(answer::draft(&crashed), Err("Nora couldn't write the reply: Access denied".to_string()));
    let gateway = NoraAnswer { id: 1, status: 502, body: "<html><body><h1>502 Bad Gateway</h1></body></html>".into() };
    assert_eq!(answer::draft(&gateway), Err("Nora couldn't write the reply: HTTP 502".to_string()));
    let empty = answer(200, json!({"message": {"success": true, "draft_text": "   "}}));
    assert_eq!(answer::draft(&empty), Err("Nora couldn't write the reply: the answer had no text".to_string()), "an empty draft is no draft");
    let unseen = answer(200, json!({"message": {"draft_text": "\u{202e}\u{0}\u{2066}"}}));
    assert_eq!(
        answer::draft(&unseen),
        Err("Nora couldn't write the reply: the answer had no text".to_string()),
        "nor one of characters that aren't seen"
    );
    let turned = answer(200, json!({"message": {"draft_text": "Bonjour\u{202e}Ada,\r\n\r\n\u{2067}Merci\u{2069}\u{7}"}}));
    assert_eq!(
        answer::draft(&turned),
        Ok("BonjourAda,\n\nMerci".to_string()),
        "no direction-changing or control character goes into the person's mail"
    );
    let not_a_message = answer(200, json!({"message": "Bonjour"}));
    assert!(answer::draft(&not_a_message).is_err(), "a draft is in a `message` object");
    // A 200 that isn't Nora's answer at all, a page of a proxy say, is no draft either.
    assert!(answer::draft(&NoraAnswer { id: 1, status: 200, body: "<html>Sign in</html>".into() }).is_err());
    // `_server_messages` as a list of objects, as other versions of Frappe send it.
    let listed = answer(403, json!({"_server_messages": [{"message": "Not permitted"}]}));
    assert_eq!(answer::draft(&listed), Err("Nora couldn't write the reply: Not permitted".to_string()));
    // Each kind of call says it in its own words.
    let said = |task: Task| task.failed("Access denied");
    assert_eq!(said(Task::Translate), "Nora couldn't translate the message: Access denied");
    assert_eq!(said(Task::Rewrite), "Nora couldn't rewrite the text: Access denied");
    assert_eq!(said(Task::Style), "Nora couldn't learn your style: Access denied");
}

#[test]
fn no_network_is_said_as_such() {
    let lost = NoraAnswer { id: 1, status: 0, body: "Failed to fetch".into() };
    assert_eq!(answer::draft(&lost), Err("Nora couldn't be reached: Failed to fetch".to_string()));
    assert_eq!(answer::translation(&lost, "fr"), Err("Nora couldn't be reached: Failed to fetch".to_string()));
    assert_eq!(answer::rewritten(&lost, "corrected_text"), Err("Nora couldn't be reached: Failed to fetch".to_string()));
    assert_eq!(answer::style_started(&lost), Err("Nora couldn't be reached: Failed to fetch".to_string()));
}

#[test]
fn a_translation_into_the_language_it_is_in_is_none() {
    let same = answer(200, json!({"message": {"translated_text": "Hello", "source_language": "en", "target_language": "en", "has_changes": false}}));
    assert_eq!(answer::translation(&same, "en"), Ok(None));
    let done =
        answer(200, json!({"message": {"translated_text": " Bonjour,\n\nLe bon à tirer est prêt. ", "source_language": "en", "has_changes": true}}));
    assert_eq!(answer::translation(&done, "fr"), Ok(Some("Bonjour,\n\nLe bon à tirer est prêt.".to_string())));
    let refused = answer(417, refusal("Unsupported target language. Supported: FR, EN, DE"));
    assert_eq!(
        answer::translation(&refused, "it"),
        Err("Nora couldn't translate the message: Unsupported target language. Supported: FR, EN, DE".to_string())
    );
}

#[test]
fn a_rewrite_reads_its_own_field_and_has_changes_false_is_none() {
    let corrected =
        answer(200, json!({"message": {"corrected_text": "Bonjour,\n\nC'est approuvé.", "improved_text": "not this one", "has_changes": true}}));
    assert_eq!(answer::rewritten(&corrected, "corrected_text"), Ok(Some("Bonjour,\n\nC'est approuvé.".to_string())));
    assert!(answer::rewritten(&corrected, "translated_text").is_err(), "another field is no rewrite");
    let unchanged = answer(200, json!({"message": {"improved_text": "Merci.", "has_changes": false}}));
    assert_eq!(answer::rewritten(&unchanged, "improved_text"), Ok(None));
    // What the server cut comes back with its mark: put over the person's text, the rest of it would go.
    let cut = answer(200, json!({"message": {"corrected_text": "Bonjour\n\n[... text truncated for processing]", "has_changes": true}}));
    assert_eq!(answer::rewritten(&cut, "corrected_text"), Err("Nora couldn't rewrite the text: the server cut the text short".to_string()));
}

#[test]
fn style_start_reads_already_running() {
    let started = answer(200, json!({"message": {"success": true, "status": "Scanning"}}));
    assert_eq!(answer::style_started(&started), Ok(false));
    let running = answer(200, json!({"message": {"success": true, "status": "Scanning", "already_running": true}}));
    assert_eq!(answer::style_started(&running), Ok(true));
    let refused = answer(417, refusal("Account is required"));
    assert_eq!(answer::style_started(&refused), Err("Nora couldn't learn your style: Account is required".to_string()));
    let odd = answer(200, json!({"message": {"success": "yes"}}));
    assert_eq!(answer::style_started(&odd), Err("Nora couldn't learn your style: the answer had no text".to_string()));
}

/// Whatever comes back, the parsers say something short on one line and never panic.
#[test]
fn hostile_answers_never_panic() {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let deep_message = format!("{{\"message\": {}{}}}", "{\"a\":".repeat(5_000), "}".repeat(5_001));
    let huge = format!("{{\"message\": {{\"draft_text\": \"{}\"}}}}", "a".repeat(10 * 1024 * 1024));
    let over = json!({"message": {"draft_text": "é".repeat(MAX_REPLY + 1), "translated_text": "é".repeat(MAX_REPLY + 1), "corrected_text": "x".repeat(MAX_REPLY + 1)}});
    let just = json!({"message": {"draft_text": "é".repeat(MAX_REPLY)}});
    let controls = json!({"_server_messages": json!([json!({"message": "No\u{0}\u{7}\r\nway\u{202E}gnirts\u{1b}[31m <script>alert(1)</script>"}).to_string()]).to_string()});
    let long_reason = json!({"exception": format!("frappe.exceptions.ValidationError: {}", "why ".repeat(10_000))});
    let bodies: Vec<String> = vec![
        String::new(),
        "null".into(),
        "[]".into(),
        "\"text\"".into(),
        "{".into(),
        "\u{0}\u{1}\u{2}".into(),
        "{\"message\": null}".into(),
        "{\"message\": []}".into(),
        "{\"message\": {\"draft_text\": 12, \"translated_text\": [], \"has_changes\": \"no\", \"success\": 1}}".into(),
        "{\"_server_messages\": 12}".into(),
        "{\"_server_messages\": \"not json\"}".into(),
        "{\"_server_messages\": \"[12, null, [], \\\"not json\\\", \\\"{\\\\\\\"message\\\\\\\": 5}\\\"]\"}".into(),
        "{\"_server_messages\": [{\"message\": \"\"}, {\"message\": \"<b></b>\"}]}".into(),
        "{\"exception\": \": \"}".into(),
        "{\"exception\": 12}".into(),
        deep,
        deep_message,
        huge,
        over.to_string(),
        controls.to_string(),
        long_reason.to_string(),
    ];
    let fits = |said: &str| {
        assert!(said.chars().count() < 600, "{} characters", said.chars().count());
        assert!(!said.chars().any(char::is_control), "{said:?}");
        assert!(!said.contains('\u{202E}'), "{said:?}");
    };
    for body in &bodies {
        for status in [0, 200, 401, 417, 500, 502, 999] {
            let a = NoraAnswer { id: 7, status, body: body.clone() };
            for said in [
                answer::draft(&a).err(),
                answer::translation(&a, "fr").err(),
                answer::rewritten(&a, "corrected_text").err(),
                answer::rewritten(&a, "translated_text").err(),
                answer::style_started(&a).err(),
            ]
            .into_iter()
            .flatten()
            {
                fits(&said);
            }
        }
    }
    // Over the length taken: refused, in the person's words.
    let a = NoraAnswer { id: 1, status: 200, body: over.to_string() };
    assert_eq!(answer::draft(&a), Err("Nora couldn't write the reply: the answer is too long".to_string()));
    assert_eq!(answer::translation(&a, "fr"), Err("Nora couldn't translate the message: the answer is too long".to_string()));
    assert_eq!(answer::rewritten(&a, "corrected_text"), Err("Nora couldn't rewrite the text: the answer is too long".to_string()));
    assert_eq!(answer::draft(&answer(200, just)).map(|d| d.chars().count()), Ok(MAX_REPLY), "the longest draft taken");
    let big = NoraAnswer { id: 1, status: 200, body: format!("{{\"message\": {{\"draft_text\": \"{}\"}}}}", "a".repeat(MAX_ANSWER)) };
    assert_eq!(answer::draft(&big), Err("Nora couldn't write the reply: the answer is too long".to_string()), "too large to be read");
    // A refusal's words with control and direction characters, and markup: one plain line.
    let said = answer::draft(&NoraAnswer { id: 1, status: 417, body: controls.to_string() }).unwrap_err();
    assert_eq!(said, "Nora couldn't write the reply: No waygnirts [31m alert(1)");
    let said = answer::draft(&NoraAnswer { id: 1, status: 500, body: long_reason.to_string() }).unwrap_err();
    assert!(said.ends_with('…') && said.chars().count() < 400, "{said}");
}

#[test]
fn text_becomes_html_without_markup() {
    assert_eq!(
        own_text::as_html("Hello <b>Ann</b> & co,\nline two\n\n\nSecond\n\n<script>alert(1)</script>"),
        "<p>Hello &lt;b&gt;Ann&lt;/b&gt; &amp; co,<br>line two</p>\n<p>Second</p>\n<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"
    );
    assert_eq!(own_text::as_html("  \n\n  "), "");
}

#[test]
fn the_context_is_cut_at_a_character_boundary() {
    let from = Address::new("Grace\r\nSubject: forged", "grace@example.org");
    let context = own_text::reply_context(&from, " The proof\nis ready ", &"é".repeat(super::CONTEXT_MAX + 1_000));
    let (head, text) = context.split_once("\n\n").unwrap();
    assert_eq!(head, "From: Grace  Subject: forged <grace@example.org>\nSubject: The proof is ready");
    assert_eq!(text.chars().count(), super::CONTEXT_MAX);
    let nameless = own_text::reply_context(&Address::new("", "grace@example.org"), "Hi", "Hello");
    assert_eq!(nameless, "From: grace@example.org\nSubject: Hi\n\nHello");
    let html = own_text::message_html(&"ü".repeat(super::CONTEXT_MAX * 2));
    assert_eq!(html.chars().count(), super::CONTEXT_MAX + "<p></p>".len());
}

/// The sample mailbox's message with this subject: a reply to it (to everyone with `all`), with the
/// account's signature unless `signature` is false.
fn reply_to(subject: &str, signature: bool) -> Session {
    let mut session = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
    if !signature {
        for account in &mut session.mailbox.accounts {
            account.signature.clear();
        }
    }
    let id = session.mailbox.messages.values().find(|m| m.subject == subject).map(|m| m.id).unwrap();
    session.run("mail.reply", &json!({"id": id})).unwrap();
    session
}

fn typed(session: &mut Session, text: &str) {
    session.run("compose.select", &json!({"from": 0})).unwrap();
    session.run("compose.type", &json!({"text": text})).unwrap();
}

fn own(session: &Session) -> OwnText {
    own_text::own_text(session.compose.as_ref().unwrap().body.text())
}

#[test]
fn own_text_stops_at_the_signature_the_dash_line_and_the_plain_quote() {
    let plain = "Q4 roadmap review moved to Thursday";
    // A reply to a plain-text message, with the signature and the original quoted in the body.
    let mut session = reply_to(plain, true);
    let body = session.compose.as_ref().unwrap().body.plain();
    assert!(body.starts_with("\n\nAvery Quinn\nLumen Studio\n\n-----Original Message-----\nFrom: Priya Nair <priya@lumen.example>"), "{body:?}");
    assert_eq!(own(&session), OwnText { text: String::new(), chars: 0 }, "nothing written yet");
    typed(&mut session, "Hello Priya,\n\nThursday works.");
    let written = "Hello Priya,\n\nThursday works.";
    assert_eq!(own(&session), OwnText { text: written.into(), chars: written.chars().count() }, "not the blank lines, the signature or the original");
    // No signature: the plain quote ends it.
    let mut session = reply_to(plain, false);
    assert!(session.compose.as_ref().unwrap().body.plain().starts_with("\n\n-----Original Message-----"));
    typed(&mut session, "Fine by me.");
    assert_eq!(own(&session).text, "Fine by me.");
    // An original in HTML is not in the body: the signature ends it, and without one nothing does.
    let html = "Your October invoice is ready";
    let mut session = reply_to(html, true);
    assert!(session.compose.as_ref().unwrap().quote.is_some());
    typed(&mut session, "Paid, thanks.");
    assert_eq!(own(&session).text, "Paid, thanks.");
    let mut session = reply_to(html, false);
    typed(&mut session, "Paid, thanks.\n\n");
    assert_eq!(own(&session), OwnText { text: "Paid, thanks.".into(), chars: 13 });
    // No marks and a signature's `-- ` line: it ends the text too.
    assert_eq!(own_text::own_text(&RichText::from_plain("Thanks!\n\n-- \nAvery Quinn")), OwnText { text: "Thanks!".into(), chars: 7 });
    assert_eq!(own_text::own_text(&RichText::from_plain("Thanks!\n--\nAvery Quinn")).text, "Thanks!");
    // Nothing at all.
    assert_eq!(own_text::own_text(&RichText::new()), OwnText::default());
    // A draft opened again has no marks: without a `-- ` line its signature reads as the person's text.
    let reopened = mailcraft_engine::mime::rich::from_html("<p>See you then.</p><p>Avery Quinn<br>Lumen Studio</p>");
    let reopened = own_text::own_text(&reopened).text;
    assert!(reopened.starts_with("See you then.") && reopened.ends_with("Avery Quinn\nLumen Studio"), "{reopened:?}");
    // A list keeps its bullets and a link its address in what Nora is given; the characters she
    // goes over are the body's own.
    let link = Style { link: Some("https://example.org/plan".into()), ..Style::default() };
    let listed = RichText::from_paragraphs(vec![
        Paragraph::new(vec![Run::plain("Two things:")], ListKind::None),
        Paragraph::new(vec![Run::plain("the "), Run::new("plan", link)], ListKind::Bullet),
        Paragraph::new(vec![Run::plain("the date")], ListKind::Bullet),
    ]);
    let own = own_text::own_text(&listed);
    assert_eq!(own.text, "Two things:\n- the plan <https://example.org/plan>\n- the date");
    assert_eq!(own.chars, listed.len());
}

/// A draft opened again whose HTML MailCraft didn't write has another's mail in its body, without
/// marks: what other programs start a quote with ends the person's text, as the server reads it too.
#[test]
fn own_text_of_a_draft_opened_again_stops_at_what_it_quotes() {
    let from_html = |html: &str| own_text::own_text(&mailcraft_engine::mime::rich::from_html(html)).text;
    for (program, html) in [
        (
            "Outlook",
            "<p>Thanks, see you then.</p><div style=\"border-top:solid #E1E1E1 1.0pt\"><p><b>From:</b> Grace Hopper &lt;grace@example.org&gt;<br><b>Sent:</b> Monday, 12 October 2026 09:00<br><b>To:</b> Avery Quinn<br><b>Subject:</b> The proof</p></div><p>Is the proof ready?</p>",
        ),
        (
            "Gmail",
            "<div dir=\"ltr\">Thanks, see you then.</div><br><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, 12 Oct 2026 at 09:00, Grace Hopper &lt;grace@example.org&gt; wrote:<br></div><blockquote class=\"gmail_quote\">Is the proof ready?</blockquote></div>",
        ),
        (
            "Thunderbird",
            "<p>Thanks, see you then.</p><div class=\"moz-cite-prefix\">-------- Original Message --------</div><p>Is the proof ready?</p>",
        ),
        (
            "a forward",
            "<p>Thanks, see you then.</p><p>---------- Forwarded message ---------<br>From: Grace Hopper<br>Date: Mon, 12 Oct 2026</p><p>Is the proof ready?</p>",
        ),
        ("Apple Mail", "<p>Thanks, see you then.</p><p>Begin forwarded message:</p><p>Is the proof ready?</p>"),
        (
            "in French",
            "<p>Thanks, see you then.</p><p>Le lun. 12 oct. 2026 à 09:00, Grace Hopper a écrit :</p><blockquote>Is the proof ready?</blockquote>",
        ),
        ("in German", "<p>Thanks, see you then.</p><p>Am 12.10.2026 um 09:00 schrieb Grace Hopper:</p><blockquote>Is the proof ready?</blockquote>"),
        ("a header in French", "<p>Thanks, see you then.</p><p>De : Grace Hopper<br>Envoyé : lundi 12 octobre 2026</p><p>Is the proof ready?</p>"),
        ("a rule", "<p>Thanks, see you then.</p><p>________________________________</p><p>Is the proof ready?</p>"),
    ] {
        assert_eq!(from_html(html), "Thanks, see you then.", "{program}");
    }
    assert_eq!(own_text::own_text(&RichText::from_plain("Thanks, see you then.\n\n> Is the proof ready?")).text, "Thanks, see you then.");
    // What reads like a header but isn't one (a line of the person's own) is theirs.
    assert_eq!(
        own_text::own_text(&RichText::from_plain("Deadline: Friday\nFrom: the printer, as agreed\nDate to keep in mind")).text,
        "Deadline: Friday\nFrom: the printer, as agreed\nDate to keep in mind"
    );
}

#[test]
fn a_text_over_the_cap_is_refused() {
    let at_most = RichText::from_plain(&"word ".repeat(MAX_OWN_TEXT / 5));
    assert!(own_text::to_rewrite(&at_most).is_ok());
    let over = RichText::from_plain(&"x".repeat(MAX_OWN_TEXT + 1));
    assert_eq!(own_text::to_rewrite(&over), Err("Your text is too long for Nora: 8000 characters at most.".to_string()));
    assert_eq!(own_text::to_rewrite(&RichText::from_plain("\n\n  \n")), Err("Write your text first: Nora works on it.".to_string()));
    // The signature and the original are not counted: only what Nora would be given.
    let mut session = reply_to("Q4 roadmap review moved to Thursday", true);
    typed(&mut session, "Thursday works.");
    assert_eq!(own_text::to_rewrite(session.compose.as_ref().unwrap().body.text()).map(|own| own.text), Ok("Thursday works.".to_string()));
}

#[test]
fn the_languages_are_three_and_named_in_the_interface() {
    assert_eq!(super::language("fr"), Some("fr"));
    assert_eq!(super::language("FR"), None, "the codes are the server's, in lower case");
    assert_eq!(super::language("it"), None);
    assert_eq!(super::language_name("de"), "German");
    assert_eq!(super::language_name("??"), "English");
    for method in super::METHODS {
        assert!(method.starts_with("nora.api.nora_webmail."), "{method}");
    }
}
