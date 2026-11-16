// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Sender images: a picture in place of a sender's initials, in the message list and over the
//! message being read.
//!
//! Off unless the owner turns it on in File › Options. Asking for a picture tells whoever keeps
//! it whose picture is looked at, so it is asked for only
//!
//! - for a sender the owner knows: one of the contacts, or somebody mail in Sent Items went to;
//! - or for mail whose pictures the owner lets come anyway: all mail (Download pictures in mail
//!   automatically), its sender's or its sender's domain's, or that message's
//!   ([`crate::reading::remote::access`]). Anybody can write to anybody, and a stranger's
//!   address, or a domain made for one recipient alone, learns when it is looked up: that is for
//!   the owner to allow, as the pictures in a stranger's mail are;
//! - only when the server that took the mail in vouches for its sender (DMARC,
//!   [`crate::reading::remote::vouched`]): anybody can write a `From:`, and a forged contact's
//!   face or a forged bank's logo would lend the forgery their trust. Mail from a server that
//!   writes no `Authentication-Results` keeps its initials;
//! - never for mail in Junk E-mail, for the same reason;
//! - only while a row of theirs is on screen, a few at a time, and once while the program runs.
//!
//! This crate opens no connections. It hands the host an address ([`Services::sender_images`])
//! and takes back bytes or nothing: the host asks Gravatar, then favicon.im, then Google,
//! through the same checked download and the same cache on disk as the pictures in mail. What
//! comes is a stranger's bytes, decoded within the same caps; an address that has no picture
//! keeps its initials.
//!
//! [`Services::sender_images`]: crate::Services::sender_images

// //// Neoffice — in a hosting page's tab (variant D's rows), every row has its round, with the sender's initials; and the icon of the
// //// Neoffice — sender's domain where the Neoffice server finds one ([`hosted`]): the host is handed the domain, not the address,
// //// Neoffice — and asks the instance, which asks the domain's own site (frappe_webmail `neomail_icons`): the browser asks no
// //// Neoffice — third party. On unless the person turns it off (`UiState::sender_logos`); never for a free mail service's domain
// //// Neoffice — ([`free_mail`]), where a person keeps their initials; never for Junk E-mail; only for a sender the person knows or
// //// Neoffice — whose server vouches for them.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Rect, Ui, vec2};
use mailcraft_model::{Address, Message};
use serde_json::{Value, json};

use crate::MailApp;
use crate::reading::images;
use crate::theme::Tokens;

/// Most addresses whose picture, or lack of one, is kept in memory (a picture is at most
/// [`SIDE`] pixels a side, 64 KB): far more than a screen of rows shows, and those on screen
/// are never let go of for another.
const MAX_KEPT: usize = 300;
/// Most addresses the host looks up at a time.
const AT_A_TIME: usize = 4;
/// Most addresses waiting for their turn.
const MAX_WANTED: usize = 64;
/// Largest picture taken, in bytes: a sender's picture is small.
const MAX_BYTES: usize = 1024 * 1024;
/// Longest side a picture is kept at, in pixels.
const SIDE: u32 = 128;
/// Longest side of a picture taken, in pixels: it is decoded whole on the window's thread.
const MAX_SIDE: u32 = 1024;
/// How long the host is waited on for one address, in milliseconds: it may try three places,
/// each for up to 30 s (`mailcraft_net::pictures::PATIENCE`).
const PATIENCE_MS: f64 = 100_000.0;

/// What is known of one address's picture.
enum Look {
    /// The host is looking (since when, in milliseconds).
    Asked(Receiver<Result<Vec<u8>, String>>, f64),
    /// Its bytes, not decoded yet: one picture is decoded in a frame.
    Came(Vec<u8>),
    Ready(egui::TextureHandle),
    /// There is none to be had: the initials stay.
    // shortcut: not asked about again until the program restarts, even when the network was down.
    Missing,
}

/// Senders' pictures, for as long as the program runs.
#[derive(Default)]
pub struct SenderImages {
    /// By address in lower case, with the frame it was last on screen in.
    looks: HashMap<String, (Look, u64)>,
    /// Addresses on screen that nothing is known of yet: asked for at the next frame.
    wanted: Vec<String>,
    /// The frame arrivals were last looked for in, and the frame a picture was last decoded in.
    ticked: Option<u64>,
    decoded: Option<u64>,
    /// How many look-ups the host had in hand at the last frame.
    flying: usize,
    /// Handed to the host with every look-up, weakly: gone with this state, it stops what is in
    /// flight before its next request.
    alive: Arc<()>,
}

impl SenderImages {
    /// Let go of the senders' pictures kept in memory (the window is out of sight,
    /// [`crate::out_of_sight`]): they are asked for again (from the host's cache first) when a
    /// row of theirs is next on screen. What is known to have no picture is kept, and is not
    /// asked about again; a look-up on its way is taken in when rows are next shown.
    pub(crate) fn let_go_of_pictures(&mut self) {
        self.looks.retain(|_, (look, _)| matches!(look, Look::Missing | Look::Asked(..)));
        self.decoded = None;
    }
}

/// Whether the owner knows `address` (in lower case): it is one of the contacts', or mail in
/// Sent Items went to it.
fn is_known(app: &mut MailApp, address: &str) -> bool {
    app.session.knows(address)
}

/// Take in what the host found, and hand it the addresses rows wanted a frame ago. Once a frame.
fn tick(app: &mut MailApp, ctx: &egui::Context) {
    let frame = images::frame(ctx);
    let offline = app.offline();
    let state = &mut app.sender_images;
    if state.ticked == Some(frame) {
        return;
    }
    state.ticked = Some(frame);
    let now = crate::now_ms();
    let mut flying = 0;
    for (look, _) in state.looks.values_mut() {
        let Look::Asked(answer, since) = look else { continue };
        *look = match answer.try_recv() {
            // A stranger's bytes: a picture within the caps, or none.
            Ok(Ok(bytes)) if bytes.len() <= MAX_BYTES && images::probe_icon(&bytes).is_some_and(|[w, h]| w.max(h) <= MAX_SIDE) => Look::Came(bytes),
            Ok(_) | Err(TryRecvError::Disconnected) => Look::Missing,
            Err(TryRecvError::Empty) if now - *since > PATIENCE_MS => Look::Missing,
            Err(TryRecvError::Empty) => {
                flying += 1;
                continue;
            }
        };
        ctx.request_repaint();
    }
    // (Working offline, nobody is asked: senders keep their initials.)
    if !offline && let Some(look_up) = &app.services.sender_images {
        // The rest is wanted again by its row, for as long as that is on screen.
        for address in std::mem::take(&mut state.wanted).into_iter().take(AT_A_TIME.saturating_sub(flying)) {
            if state.looks.contains_key(&address) {
                continue;
            }
            trim(&mut state.looks, frame);
            let answer = look_up(address.clone(), Arc::downgrade(&state.alive));
            state.looks.insert(address, (Look::Asked(answer, now), frame));
            flying += 1;
        }
    }
    state.flying = flying;
    if flying > 0 {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// Forget what was on screen longest ago while there is too much; never what is on its way.
fn trim(looks: &mut HashMap<String, (Look, u64)>, frame: u64) {
    while looks.len() >= MAX_KEPT {
        let oldest = looks
            .iter()
            .filter(|(_, (look, used))| *used < frame && !matches!(look, Look::Asked(..)))
            .min_by_key(|(_, (_, used))| *used)
            .map(|(address, _)| address.clone());
        match oldest {
            Some(address) => drop(looks.remove(&address)),
            None => return,
        }
    }
}

// //// Neoffice — a hosting page's tab: what its rows show (`hosted`), and the domains of free mail services, whose icon is never asked.
/// Whether the window is a hosting page's tab, where the Neoffice server looks for the icon of a sender's domain.
pub(crate) fn hosted(app: &MailApp) -> bool {
    !app.session.keeps_local
}

/// Whether senders' pictures are shown: in a hosting page's tab the icons the Neoffice server finds (`UiState::sender_logos`, on
/// unless turned off), elsewhere upstream's switch (`UiState::sender_images`, off unless turned on).
pub(crate) fn on(app: &MailApp) -> bool {
    if hosted(app) { app.ui.sender_logos } else { app.ui.sender_images }
}

/// The domains of free mail services: what follows their `@` stands for a service, not for whoever writes; their senders keep
/// their initials. A domain of these names under another country's ending is one too (`yahoo.fr`, `gmx.at`).
const FREE_MAIL_NAMES: &[&str] = &[
    "gmail",
    "googlemail",
    "yahoo",
    "ymail",
    "outlook",
    "hotmail",
    "live",
    "msn",
    "icloud",
    "me",
    "mac",
    "aol",
    "gmx",
    "web",
    "bluewin",
    "sunrise",
    "hispeed",
    "swissonline",
    "bluemail",
    "proton",
    "protonmail",
    "pm",
    "tutanota",
    "tuta",
    "infomaniak",
    "ik",
    "yopmail",
    "orange",
    "free",
    "wanadoo",
    "laposte",
    "sfr",
    "libero",
    "virgilio",
    "t-online",
    "freenet",
    "yandex",
    "mail",
    "zoho",
    "seznam",
    "posteo",
    "mailbox",
    "fastmail",
    "hey",
    "duck",
];

/// Whether `domain` (lower case) is a free mail service's: one of [`FREE_MAIL_NAMES`] and an ending of one part, or of two
/// that are a country's (`gmail.com`, `yahoo.co.uk`).
pub(crate) fn free_mail(domain: &str) -> bool {
    let parts: Vec<&str> = domain.trim_end_matches('.').split('.').collect();
    match parts.as_slice() {
        [name, _] => FREE_MAIL_NAMES.contains(name),
        // (A country's ending of two parts, `co.uk`; not a company's site under its own domain, `mail.company.ch`.)
        [name, second, country] => ["co", "com", "net", "org", "ne", "or"].contains(second) && country.len() == 2 && FREE_MAIL_NAMES.contains(name),
        _ => false,
    }
}

/// The site `domain` is under, whose icon stands for it: its last two names (`example.com` for `news.example.com`), or its last
/// three under a country's ending of two (`example.co.uk`).
pub(crate) fn site_of(domain: &str) -> String {
    let parts: Vec<&str> = domain.split('.').collect();
    let country = parts.len() >= 3 && ["co", "com", "net", "org", "ne", "or"].contains(&parts[parts.len() - 2]) && parts[parts.len() - 1].len() == 2;
    let keep = if country { 3 } else { 2 };
    parts[parts.len().saturating_sub(keep)..].join(".")
}

/// The domain of `address` (lower case, what follows its last `@`), `None` where it has none that could be a site's.
pub(crate) fn domain_of(address: &str) -> Option<String> {
    let domain = address.trim().rsplit_once('@')?.1.trim().trim_end_matches('.').to_ascii_lowercase();
    let plain = domain.contains('.')
        && domain.len() <= 253
        && domain.split('.').all(|label| !label.is_empty() && label.len() <= 63)
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        // (A name, not an address written as numbers.)
        && domain.rsplit('.').next().is_some_and(|ending| ending.len() >= 2 && ending.chars().all(|c| c.is_ascii_alphabetic()));
    plain.then_some(domain)
}

/// The picture to show for `who`, on the mail `m`, in place of their initials. `None` while
/// there is none; one that is still to be looked up is noted, and asked for at the next frame.
pub fn picture(app: &mut MailApp, ctx: &egui::Context, m: &Message, who: &Address) -> Option<egui::TextureHandle> {
    // //// Neoffice — (`on`: in a hosting page's tab, its own switch.)
    if !on(app) || app.services.sender_images.is_none() {
        return None;
    }
    if app.session.mailbox.in_junk(m.folder) {
        return None;
    }
    tick(app, ctx);
    let address = who.email.trim().to_lowercase();
    // The say-so for a message's pictures is its sender's: whoever it goes to must be known.
    let sender = address == m.from.email.trim().to_lowercase();
    // //// Neoffice — in a hosting page's tab, the icon of its site, which its server fetches from that site: for a sender the person
    // //// Neoffice — knows, or whose pictures they chose to download (as upstream's), never for a free mail service's domain; asked by
    // //// Neoffice — the site (`site_of`: a name made for one recipient alone tells nothing), its key here. (Who vouches for a sender
    // //// Neoffice — can't be read where the window's mail server isn't the one that wrote the mail's verdict: a forged sender the
    // //// Neoffice — person knows shows that sender's logo, as it shows their initials.)
    let key = if hosted(app) {
        let site = domain_of(&address).map(|d| site_of(&d)).filter(|s| !free_mail(s))?;
        if !is_known(app, &address) && !(sender && crate::reading::remote::access(app, m) == crate::reading::Access::Allowed) {
            return None;
        }
        site
    } else {
        // Who it went to is the owner's word (it is theirs); who sent it, only the server's.
        if sender && !crate::reading::remote::vouched(app.session.mailbox.account(m.account), m) {
            return None;
        }
        if !is_known(app, &address) && !(sender && crate::reading::remote::access(app, m) == crate::reading::Access::Allowed) {
            return None;
        }
        address
    };
    let address = key;
    let frame = images::frame(ctx);
    let state = &mut app.sender_images;
    let Some((look, used)) = state.looks.get_mut(&address) else {
        if state.wanted.len() < MAX_WANTED && !state.wanted.contains(&address) {
            state.wanted.push(address);
            // With every place taken, the 250 ms repaint of `tick` comes back for it.
            if state.flying < AT_A_TIME {
                ctx.request_repaint();
            }
        }
        return None;
    };
    *used = frame;
    if let Look::Came(bytes) = look {
        if state.decoded == Some(frame) {
            ctx.request_repaint();
            return None;
        }
        state.decoded = Some(frame);
        *look = match images::decode_icon(bytes, SIDE) {
            Some(image) => Look::Ready(ctx.load_texture("mailcraft-sender-image", image, egui::TextureOptions::LINEAR)),
            None => Look::Missing,
        };
    }
    match look {
        Look::Ready(texture) => Some(texture.clone()),
        _ => None,
    }
}

/// `who`'s round avatar on the mail `m`: their picture when there is one, else their initials.
pub fn avatar(app: &mut MailApp, ui: &Ui, center: egui::Pos2, radius: f32, m: &Message, who: &Address) {
    let Some(texture) = picture(app, ui.ctx(), m, who) else {
        crate::widgets::avatar(ui, center, radius, who);
        return;
    };
    // On white, as a browser shows an icon: one with see-through parts stays readable in the dark.
    ui.painter().circle_filled(center, radius, crate::theme::Tokens::get(ui.ctx()).avatar_backdrop);
    egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
        .corner_radius(radius)
        .paint_at(ui, Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0)));
}

/// The row of File › Options, saying who is asked and what each is told.
pub fn options(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    // //// Neoffice — a hosting page's tab: the icons its Neoffice server fetches, and what a sender's site learns of it.
    if hosted(app) {
        let mut on = app.ui.sender_logos;
        if ui.checkbox(&mut on, tl!("Show the logos of senders' companies")).changed() {
            let _ = app.run("ui.senderImages", json!({"value": on}));
        }
        let said = tl!(
            "Your Neoffice server asks the website of the sender's domain for its icon and keeps it a week, for everyone of your company. That site learns that your company's server asked for its icon, not who you are nor which message. Never for free mail services (Gmail, Outlook.com, Bluewin…), whose senders keep their initials, never for Junk E-mail, and only for the people you know or whose pictures you chose to download."
        );
        ui.label(egui::RichText::new(said).color(t.text_dim));
        return;
    }
    let mut on = app.ui.sender_images;
    if ui.checkbox(&mut on, tl!("Show sender images from Gravatar, favicon.im and Google")).changed() {
        let _ = app.run("ui.senderImages", json!({"value": on}));
    }
    let said = tl!(
        "Gravatar is asked for the sender's picture and told a hash of their address; else favicon.im, then Google, are asked for their website's icon and told the sender's domain. This tells those services whose pictures you look at. Only for the people you know (your contacts, and whoever you have written to) and the mail whose pictures you download, never for Junk E-mail, and a sender's picture only when your mail server confirms the mail is from them."
    );
    ui.label(egui::RichText::new(said).color(t.text_dim));
}

/// `ui.senderImages`; forgetting what is in memory when it is turned off.
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    (id == "ui.senderImages").then(|| {
        let on = p.get("value").and_then(Value::as_bool).unwrap_or(!self::on(app));
        // //// Neoffice — in a hosting page's tab, its own switch (`on`).
        if hosted(app) {
            app.ui.sender_logos = on;
        } else {
            app.ui.sender_images = on;
        }
        if !on {
            app.sender_images = SenderImages::default();
        }
        Ok(json!({"value": on}))
    })
}

#[cfg(test)]
#[path = "sender_images_tests.rs"]
mod tests;
