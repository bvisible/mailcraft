// //// Neoffice — added file (no upstream equivalent): the signature a hosting page gives a mailbox, its logo by its bytes.

//! The signature a hosting page gives a mailbox it hands over (host.rs), set up in the session MailCraft starts
//! with ([`set_up`]): its HTML, or its text, and for a company's signature its logo by its bytes and the place the
//! company's layout gives it. MailCraft keeps a signature's pictures by their bytes and takes none from its HTML
//! (`signature.set`), so the page hands the logo itself, and it goes in with `signature.picture.add`,
//! `signature.picture.set` and `signature.layout`, as the owner would put it in. Built everywhere, so that it is
//! tested natively.
// Outside the browser only the tests run it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use mailcraft_engine::Session;
use mailcraft_model::signature::{MAX_ALT, MAX_PICTURE_BYTES, MAX_WIDTH};
use serde_json::json;

/// The longest logo a page gives, in base64: a picture as big as a signature's may be.
pub const MAX_LOGO_BASE64: usize = MAX_PICTURE_BYTES.div_ceil(3) * 4;
/// The longest name of a logo's file, in characters.
const MAX_LOGO_NAME: usize = 100;
/// The places a page may give a company's logo (`signature.layout`).
const LAYOUTS: [&str; 2] = ["pictureLeft", "pictureAbove"];

/// A logo a page gives with a signature: its bytes in base64, its file's name, its description, and the width it is
/// shown at.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PagePicture {
    pub data: String,
    pub name: String,
    pub alt: String,
    pub width: Option<u16>,
}

impl std::fmt::Debug for PagePicture {
    // Its size, never its bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PagePicture").field("base64", &self.data.len()).field("width", &self.width).finish_non_exhaustive()
    }
}

/// The signature a page gives a mailbox: its HTML and its text (either may be empty), and a company's logo with the
/// place it takes (`pictureLeft`, `pictureAbove`; empty: none).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PageSignature {
    pub html: String,
    pub text: String,
    pub logo: Option<PagePicture>,
    pub layout: String,
}

impl std::fmt::Debug for PageSignature {
    // Sizes, never the words: a signature holds a person's name, numbers and address, and debug output may reach a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageSignature")
            .field("html", &self.html.len())
            .field("text", &self.text.len())
            .field("logo", &self.logo)
            .field("layout", &self.layout)
            .finish()
    }
}

/// What a page says of a logo, kept to what MailCraft takes: none without bytes, or with more than a signature's
/// picture may have; a name and a description cut to their lengths; a width of 1 to `MAX_WIDTH` pixels, or none.
/// (Whether the bytes are a picture is the engine's to say: `signature.picture.add` reads them.)
pub fn picture(data: &str, name: &str, alt: &str, width: Option<f64>) -> Option<PagePicture> {
    let data = data.trim();
    if data.is_empty() || data.len() > MAX_LOGO_BASE64 {
        return None;
    }
    Some(PagePicture {
        data: data.to_string(),
        name: name.trim().chars().take(MAX_LOGO_NAME).collect(),
        alt: alt.trim().chars().take(MAX_ALT).collect(),
        width: width.filter(|w| (1.0..=f64::from(MAX_WIDTH)).contains(w)).map(|w| w.round() as u16),
    })
}

/// The place a page gives a logo, if MailCraft knows it.
pub fn layout(given: &str) -> String {
    LAYOUTS.iter().find(|known| **known == given.trim()).map_or_else(String::new, |known| (*known).to_string())
}

/// Make `signature` the signature of `account` (named after its address, `email`), for its new messages and its
/// replies and forwards: its HTML, else its text, then its logo at the start and the logo's place. Whether a signature
/// was made. A logo the engine does not take (no picture by its own bytes) leaves the signature without it; nothing
/// here stops the mailbox from opening.
pub fn set_up(session: &mut Session, account: u64, email: &str, signature: &PageSignature) -> bool {
    let mut made = json!({"name": email, "default": true, "account": account});
    if !signature.html.trim().is_empty() {
        made["html"] = signature.html.clone().into();
    } else if !signature.text.trim().is_empty() {
        made["text"] = signature.text.clone().into();
    } else {
        return false;
    }
    if session.run("signature.set", &made).is_err() {
        log::warn!("the signature of a mailbox the page handed over couldn't be set");
        return false;
    }
    let Some(logo) = &signature.logo else { return true };
    let added = session.run("signature.picture.add", &json!({"name": email, "data": logo.data, "file": logo.name, "alt": logo.alt}));
    let Some(id) = added.ok().and_then(|answer| answer["id"].as_u64()) else {
        log::warn!("the logo of a signature the page handed over couldn't be added");
        return true;
    };
    if let Some(width) = logo.width
        && session.run("signature.picture.set", &json!({"name": email, "id": id, "width": width})).is_err()
    {
        log::warn!("the width of a signature's logo couldn't be set");
    }
    if !signature.layout.is_empty() && session.run("signature.layout", &json!({"name": email, "value": signature.layout})).is_err() {
        log::warn!("the place of a signature's logo couldn't be set");
    }
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use mailcraft_model::Mailbox;
    use mailcraft_model::signature::Layout;

    use super::*;

    const ME: &str = "ada@example.org";
    /// A PNG of one pixel.
    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    /// A company's signature as the page writes it: the logo in a cell, the words in the next.
    const COMPANY: &str = r#"<table><tr><td width="90"><img src="/files/logo.png" alt="Example Ltd" width="90"></td><td><div><b>Ada Lovelace</b></div><div>Example Ltd</div></td></tr></table>"#;

    fn hosted() -> (Session, u64) {
        let mut session = Session::new(Mailbox::with_local_account("", ""));
        let added = session
            .run("account.add", &json!({"kind": "jmap", "email": ME, "displayName": "Ada", "username": ME, "jmap": {"session": "https://mail.example.org/jmap/session", "bearer": true}}))
            .unwrap();
        let id = added["id"].as_u64().unwrap();
        (session, id)
    }

    fn company(logo: Option<PagePicture>, place: &str) -> PageSignature {
        PageSignature { html: COMPANY.to_string(), text: "Ada Lovelace\nExample Ltd".to_string(), logo, layout: layout(place) }
    }

    #[test]
    fn a_company_signature_comes_with_its_logo_beside_its_words() {
        let (mut session, id) = hosted();
        let logo = picture(PIXEL, "logo.png", "Example Ltd", Some(90.0));
        assert!(set_up(&mut session, id, ME, &company(logo, "pictureLeft")));
        let signature = session.mailbox.signature(ME).unwrap();
        assert_eq!(signature.layout, Layout::PictureLeft);
        let pictures = signature.pictures();
        assert_eq!(pictures.len(), 1, "the logo, and nothing taken from the HTML");
        assert_eq!((pictures[0].width, pictures[0].alt.as_str(), pictures[0].mime.as_str()), (Some(90), "Example Ltd", "image/png"));
        assert!(signature.plain().contains("Ada Lovelace") && signature.plain().contains("Example Ltd"), "{}", signature.plain());
        // New messages, replies and forwards start with it.
        let account = session.mailbox.account(id).unwrap();
        assert_eq!((account.signature.as_str(), account.reply_signature.as_deref()), (ME, Some(ME)));
    }

    #[test]
    fn a_logo_above_the_words_and_one_at_its_own_width() {
        let (mut session, id) = hosted();
        assert!(set_up(&mut session, id, ME, &company(picture(PIXEL, "logo.png", "", None), "pictureAbove")));
        let signature = session.mailbox.signature(ME).unwrap();
        assert_eq!((signature.layout, signature.pictures()[0].width), (Layout::PictureAbove, None));
    }

    #[test]
    fn a_logo_that_is_no_picture_leaves_the_signature_without_it() {
        let (mut session, id) = hosted();
        let junk = picture("bm90IGEgcGljdHVyZQ==", "logo.png", "", Some(90.0));
        assert!(set_up(&mut session, id, ME, &company(junk, "pictureLeft")));
        let signature = session.mailbox.signature(ME).unwrap();
        assert!(signature.pictures().is_empty());
        assert_eq!(signature.layout, Layout::Inline, "no place for a logo that isn't there");
        assert!(signature.plain().contains("Ada Lovelace"));
    }

    #[test]
    fn a_signature_without_a_logo_is_its_html_or_its_text_and_nothing_is_none() {
        let (mut session, id) = hosted();
        let own = PageSignature { html: "<p>Ada</p>".into(), ..PageSignature::default() };
        assert!(set_up(&mut session, id, ME, &own));
        assert_eq!(session.mailbox.signature(ME).unwrap().plain().trim(), "Ada");
        let (mut session, id) = hosted();
        let text = PageSignature { text: "Ada, by text".into(), ..PageSignature::default() };
        assert!(set_up(&mut session, id, ME, &text));
        assert_eq!(session.mailbox.signature(ME).unwrap().plain().trim(), "Ada, by text");
        let (mut session, id) = hosted();
        assert!(!set_up(&mut session, id, ME, &PageSignature::default()));
        assert!(session.mailbox.signature(ME).is_none());
    }

    #[test]
    fn what_a_page_says_of_a_logo_is_kept_to_what_mailcraft_takes() {
        assert_eq!(picture("", "logo.png", "", None), None, "no bytes");
        assert_eq!(picture(&"A".repeat(MAX_LOGO_BASE64 + 4), "logo.png", "", None), None, "too big");
        let kept = picture(PIXEL, &"n".repeat(300), &"a".repeat(300), Some(90.4)).unwrap();
        assert_eq!((kept.name.chars().count(), kept.alt.chars().count(), kept.width), (MAX_LOGO_NAME, MAX_ALT, Some(90)));
        for width in [0.0, -5.0, f64::from(MAX_WIDTH) + 1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(picture(PIXEL, "", "", Some(width)).unwrap().width, None, "{width}");
        }
        assert_eq!(layout("pictureLeft"), "pictureLeft");
        assert_eq!(layout(" pictureAbove "), "pictureAbove");
        for unknown in ["inline", "left", "PictureLeft", "", "javascript:"] {
            assert_eq!(layout(unknown), "", "{unknown}");
        }
        // The bytes and the words stay out of debug output, which may reach a log.
        assert!(!format!("{kept:?}").contains(PIXEL));
        let shown = format!("{:?}", company(Some(kept), "pictureLeft"));
        assert!(!shown.contains("Ada") && !shown.contains(PIXEL), "{shown}");
    }
}
