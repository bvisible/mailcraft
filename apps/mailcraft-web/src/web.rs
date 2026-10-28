// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The browser shell: web `Services` (file picker, downloads, storage, JMAP), drag-and-drop and
//! the eframe web runner.

// //// Neoffice — (no `Rc` here: it held upstream's page hook, left out below)

use mailcraft_engine::Session;
use mailcraft_model::Mailbox;
use mailcraft_ui_egui::opening::{Arrival, Trouble};
use mailcraft_ui_egui::{Inbox, MailApp, Services, UiState};
use wasm_bindgen::JsCast as _;

use crate::page;

const MAILBOX_EXTS: &[&str] = &["mailcraft", "json"];
const IMPORT_EXTS: &[&str] = &["eml", "mbox", "mbx", "ics", "ical", "vcf", "vcard"];
const CANVAS_ID: &str = "mailcraft_canvas";
const LOADING_ID: &str = "mailcraft_loading";
/// Where the mailbox and the interface settings live in the browser's storage.
const MAILBOX_KEY: &str = "mailcraft.mailbox.v1";
const UI_KEY: &str = "mailcraft.ui.v1";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    // //// Neoffice — the name the hosting page gives the program (host.rs) is the one every interface string shows.
    if let Some(name) = crate::host::product() {
        mailcraft_ui_egui::i18n::set_product_name(&name);
    }
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        // //// Neoffice — (upstream's page hook for JMAP accounts, `jmap.rs`, is left out: a hosting page's mailboxes are read
        // //// Neoffice — by host.rs and run by jmap_services.rs)
        let mut options = eframe::WebOptions::default();
        // //// Neoffice — and a browser whose WebGPU gives no adapter in time starts on WebGL2 at once, rather than waiting
        // //// Neoffice — for an answer that may never come (gpu_probe.rs).
        if (query().contains("webgl") || !crate::gpu_probe::answers().await)
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("mailcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    // //// Neoffice — the mailboxes a hosting page hands over open by themselves (hosted_app), in place of the
                    // //// Neoffice — mailbox kept in this browser and of the opening screen that reads it.
                    let hosted = crate::host::accounts();
                    let app = if !hosted.is_empty() {
                        // //// Neoffice — and the page asks before it is left while a note's change waits (hosted/leave.rs).
                        crate::hosted::leave::install();
                        let mut app = hosted_app(&hosted, inbox.clone(), cc.egui_ctx.clone());
                        if let Some(ui) = stored(UI_KEY).and_then(|text| serde_json::from_str::<UiState>(&text).ok()) {
                            app.apply_prefs(ui);
                        }
                        // //// Neoffice — (its mailboxes were added by hosted_app, not by upstream's page hook)
                        app
                    } else {
                        let prefs = stored(UI_KEY).and_then(|text| serde_json::from_str::<UiState>(&text).ok());
                        // The window is there at once, and the mailbox is read from the browser's
                        // storage after its first frame (#519): the same opening screen as on the
                        // desktop, which says "Opening…" alone, for the storage says nothing of how far
                        // it has got.
                        let (tx, rx) = std::sync::mpsc::channel();
                        let (ctx, wake, first_prefs, build_inbox) = (cc.egui_ctx.clone(), cc.egui_ctx.clone(), prefs.clone(), inbox.clone());
                        // //// Neoffice — a JMAP account is read by this fork's runner (jmap_services.rs, in `services`), not by
                        // //// Neoffice — upstream's page hook (`jmap.rs`, which this fork leaves out).
                        let build: mailcraft_ui_egui::opening::Build = Box::new(move |arrival| {
                            let mut app = MailApp::new(Session::new(arrival.mailbox), services(build_inbox, ctx.clone()));
                            if let Some(ui) = prefs {
                                app.apply_prefs(ui);
                            }
                            // //// Neoffice — (no page hook's accounts added here: see above)
                            app
                        });
                        wasm_bindgen_futures::spawn_local(async move {
                            after_a_frame().await;
                            // //// Neoffice — (no page hook naming accounts: see above)
                            let _ = tx.send(first_mailbox(false));
                            wake.request_repaint();
                        });
                        let opening = mailcraft_ui_egui::opening::Opening::new(rx, Default::default(), build);
                        let locale = web_sys::window().and_then(|w| w.navigator().language());
                        MailApp::opening(opening, first_prefs, locale)
                    };
                    let saved_ui = serde_json::to_string(&app.prefs()).unwrap_or_default();
                    Ok(Box::new(WebShell { app, inbox, saved_ui, checked: 0.0 }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                // //// Neoffice — under the name the page that loads it says (index.html): NeoMail, this build being Neoffice's.
                Err(e) => el.set_inner_html(&format!("<p>NeoMail failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn stored(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

fn sample() -> Mailbox {
    let now = mailcraft_time::now();
    mailcraft_engine::sample::mailbox(now, mailcraft_time::local_offset_minutes(now))
}

/// The mailbox to start with: what the URL asks for, else the one kept in this browser, else the
/// sample (an empty mail program in a browser tab has nothing to show), or an empty one when the
/// page names accounts of its own (`named`), which are added to it. One kept here that can't be
/// read says why before the window carries on with the sample or an empty one, as it always did.
fn first_mailbox(named: bool) -> Arrival {
    let q = query();
    if page::has_flag(&q, "empty") {
        return Arrival::opened(Mailbox::with_local_account("", ""), None);
    }
    if page::has_flag(&q, "sample") {
        return Arrival::opened(sample(), None);
    }
    let fresh = || if named { Mailbox::with_local_account("", "") } else { sample() };
    match stored(MAILBOX_KEY).map(|text| mailcraft_engine::io::read_mailbox(text.as_bytes())) {
        Some(Ok(mailbox)) => Arrival::opened(mailbox, None),
        Some(Err(e)) => {
            log::error!("the stored mailbox couldn't be read: {e}");
            Arrival { trouble: Some(Trouble::Unreadable(e.to_string())), ..Arrival::opened(fresh(), None) }
        }
        None => Arrival::opened(fresh(), None),
    }
}

/// Resolved once the browser has had a moment to show the page (a frame, as a rule).
async fn after_a_frame() {
    let tick = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 30);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(tick).await;
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously),
/// feed them through the inbox, and keep the interface settings in the browser's storage.
struct WebShell {
    app: MailApp,
    inbox: Inbox,
    /// The interface settings as last stored.
    saved_ui: String,
    checked: f64,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        self.app.logic(ctx);
        // //// Neoffice — whether leaving the page now would lose a note's change (hosted/leave.rs).
        crate::hosted::leave::set(self.app.notes_unsent());
        // //// Neoffice — a hosting page's window tells the page that frames it whether closing it now would lose work (host_dirty.rs).
        if !self.app.session.keeps_local {
            crate::host_dirty::tell(self.app.unsent_work());
        }
        // //// Neoffice — a page that draws the navigation in its own menu is told where the window is, and its menu's commands are run (host_menu.rs).
        // //// Neoffice — and for modules the page draws itself (host_modules.rs of the window), whose messages it hears.
        if self.app.services.host_menu.is_some() || self.app.services.modules.is_some() {
            crate::host_menu::frame(&mut self.app, ctx);
        }
        open_links(ctx);
        // Interface settings: stored when they change, checked every couple of seconds.
        let now = mailcraft_ui_egui::now_ms();
        if now - self.checked > 2000.0 {
            self.checked = now;
            if let Ok(text) = serde_json::to_string(&self.app.prefs())
                && text != self.saved_ui
            {
                if let Some(s) = storage() {
                    let _ = s.set_item(UI_KEY, &text);
                }
                self.saved_ui = text;
            }
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        open_links(ui.ctx());
    }
}

/// Open the links the window followed in this frame, in place of eframe, which would open them
/// with a handle on this page left in the new one (`window.opener`): in a new tab with
/// `noopener,noreferrer`, and only `http`, `https`, `mailto` and `tel` (`page::web_link`).
fn open_links(ctx: &egui::Context) {
    let links = ctx.output_mut(|o| {
        let mut links = Vec::new();
        o.commands.retain(|c| match c {
            egui::OutputCommand::OpenUrl(open) => {
                links.push((open.url.clone(), open.new_tab));
                false
            }
            _ => true,
        });
        links
    });
    let Some(window) = web_sys::window() else { return };
    for (url, new_tab) in links {
        let Some(url) = page::web_link(&url) else { continue };
        let _ = if new_tab {
            window.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer")
        } else {
            window.open_with_url_and_target(&url, "_self")
        };
    }
}

fn services(inbox: Inbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    // //// Neoffice — an account kept in step with a JMAP server is read through `fetch` (jmap_services.rs), with the credential its owner types; a hosting page's credentials replace these (hosted_app).
    let (jmap_ctx, gate) = (ctx.clone(), crate::jmap_services::Gate::default());
    let no_credentials = std::rc::Rc::new(crate::credentials::NoCredentials);
    // //// Neoffice — the pictures on the web a message links to, through the hosting page when it fetches them (host.rs): a browser tab can't read another site's bytes itself.
    let pictures = crate::host::fetches_urls().then(|| pictures(ctx.clone()));
    // //// Neoffice — Nora, the assistant of the Neoffice server, when the hosting page turns her on (nora.rs); the mailboxes it handed over are named in hosted_app;
    // //// Neoffice — her pane's conversation where its calls can go (`nora::chat`), and the pages of the instance her cards lead to where the page opens them (`nora::opener`).
    let nora = crate::nora::service(ctx.clone()).map(|call| mailcraft_ui_egui::nora::Host {
        call,
        mailboxes: Vec::new(),
        chat: crate::nora::chat(),
        open: crate::nora::opener(),
    });
    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = match purpose {
                "mailbox" => rfd::AsyncFileDialog::new().add_filter("MailCraft mailbox", MAILBOX_EXTS),
                "import" => rfd::AsyncFileDialog::new().add_filter("Mail, calendar and contact files", IMPORT_EXTS),
                // Anything can be attached.
                _ => rfd::AsyncFileDialog::new(),
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        download: Some(Box::new(download)),
        inbox: Some(inbox),
        persist: Some(Box::new(persist)),
        // //// Neoffice — as above.
        jmap: Some(crate::jmap_services::jmap_service(jmap_ctx.clone(), no_credentials.clone(), gate.clone())),
        jmap_fetch: Some(crate::jmap_services::jmap_fetch_service(jmap_ctx, no_credentials, gate)),
        // //// Neoffice — as above.
        pictures,
        // //// Neoffice — as above.
        nora,
        // The browser's language, for an interface language left on Automatic; and the page says
        // which language it is in (`<html lang>`), for screen readers and the browser's own tools.
        // //// Neoffice — the hosting page's language first (the person's, host.rs), else the browser's.
        system_locale: crate::host::language().or_else(|| web_sys::window().and_then(|w| w.navigator().language())),
        on_language: Some(Box::new(|code: &str| {
            let root = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element());
            if let Some(root) = root {
                let _ = root.set_attribute("lang", code);
            }
        })),
        ..Default::default()
    }
}

// //// Neoffice — the largest picture taken from the hosting page, the reading pane's own limit.
const MAX_PICTURE_BYTES: usize = 8 * 1024 * 1024;

// //// Neoffice — the pictures service of a page that fetches them (host.rs `fetch_url`).
/// One picture through the hosting page: its bytes, or why there are none, on the channel the reading pane looks at,
/// and the window woken when they come. The reading pane decides what is asked for and when (only on the owner's
/// say-so, a few at a time), and looks at what comes as warily as at an attachment.
fn pictures(ctx: egui::Context) -> Box<dyn Fn(String) -> std::sync::mpsc::Receiver<Result<Vec<u8>, String>>> {
    Box::new(move |url: String| {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = crate::host::fetch_url(&url, MAX_PICTURE_BYTES).await.unwrap_or_else(|| Err("the page fetches no pictures".into()));
            // The reading pane may have stopped waiting (the message left the screen): nothing to tell then.
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
        rx
    })
}

// //// Neoffice — the mail program of a hosting page: the mailboxes it handed over, kept in step with their JMAP servers with its credentials, and nothing kept between visits.
/// The mail program of a hosting page (host.rs): the mailboxes it handed over and nothing else, no sample. Their
/// mail is their servers', read again at each visit; their credentials are the page's, in this tab's memory
/// only, and renewed by it. Nothing of them is kept in this browser: only the interface settings are.
fn hosted_app(accounts: &[crate::host::HostAccount], inbox: Inbox, ctx: egui::Context) -> MailApp {
    let mut session = Session::new(Mailbox::with_local_account("", ""));
    for a in accounts {
        let added = session.run(
            "account.add",
            &serde_json::json!({
                "kind": "jmap",
                "email": a.email,
                "displayName": a.name,
                "username": a.username,
                "jmap": {"session": a.session, "bearer": a.bearer},
            }),
        );
        // Neither the address nor the server's goes into the console.
        let Some(id) = added.ok().and_then(|v| v["id"].as_u64()) else {
            log::error!("a mailbox the page handed over couldn't be set up");
            continue;
        };
        // Its signature, as the page gives it, with a company's logo by its bytes: new messages, replies and forwards
        // start with it (page_signature.rs).
        crate::page_signature::set_up(&mut session, id, &a.email, &a.signature);
    }
    // What set the page's mailboxes up is no change of the owner's: nothing to undo.
    session.forget_history();
    // A clock of 24 hours, but for a page in English.
    if crate::host::language().is_some_and(|tag| !tag.to_ascii_lowercase().starts_with("en")) {
        session.prefs.twelve_hour = false;
    }
    let mut services = services(inbox, ctx.clone());
    // //// Neoffice — File › Signatures links to where the page's site writes these mailboxes' signatures (hosted_signatures.rs of the window).
    services.signatures_home = crate::host::signatures();
    let credentials = std::rc::Rc::new(crate::host::HostCredentials::new(accounts));
    let gate = crate::jmap_services::Gate::default();
    // //// Neoffice — a message of these mailboxes goes to Nora by its id on the server, never its text, and their style can be learnt there (nora.rs); each mailbox by the name of the server's account for it, which is not its address once that changed.
    if let Some(nora) = &mut services.nora {
        nora.mailboxes = accounts.iter().map(crate::nora::handed_mailbox).collect();
    }
    // //// Neoffice — a page that draws the navigation in its own menu (host.rs `chrome`): no module bar nor folder pane, and the mailboxes it handed over, in its order, for what it is told and what its menu asks (host_menu.rs).
    if crate::host::menu() {
        let mailboxes = accounts.iter().map(|a| mailcraft_ui_egui::host_menu::Handed { email: a.email.clone(), name: a.name.clone() }).collect();
        services.host_menu = Some(mailcraft_ui_egui::host_menu::Menu::new(mailboxes));
        crate::host_menu::listen(ctx.clone());
    }
    // //// Neoffice — a page that takes a message to share in its team chat (host.rs `share`): Share in Synk and Chat in Synk hand it the message of one of these mailboxes, by its mailbox and its id on the server (host_share.rs of the window).
    // //// Neoffice — modules the page draws itself (host.rs `modules`): their buttons in the module bar, the window's place left them, and the page's messages heard (host_modules.rs of the window).
    let modules = crate::host::modules();
    if !modules.is_empty() {
        services.modules = Some(mailcraft_ui_egui::host_modules::Modules::new(&modules, Box::new(crate::host_menu::tell)));
        if services.host_menu.is_none() {
            crate::host_menu::listen(ctx.clone());
        }
    }
    if crate::host::share() {
        services.share = Some(mailcraft_ui_egui::host_share::Share {
            mailboxes: accounts.iter().map(|a| a.email.clone()).collect(),
            // //// Neoffice — the page is told what host_share.rs makes of a click (a share, a chat), as it is.
            post: Box::new(crate::host_menu::tell),
        });
    }
    // //// Neoffice — a page that does things with an attachment (host.rs `attachments`): Save to Drive…, Send to Scanned Documents and Open in its editor name an attachment of one of these mailboxes to it, by its message's mailbox and server id and what identifies the file (host_attachments.rs of the window).
    let offered = crate::host::attachments();
    if !offered.is_empty() {
        services.attachments = Some(mailcraft_ui_egui::host_attachments::Attachments {
            mailboxes: accounts.iter().map(|a| a.email.clone()).collect(),
            actions: offered,
            post: Box::new(crate::host_menu::tell),
        });
    }
    // //// Neoffice — their mail, their downloads and their calendars, read with the page's credentials through the page's one gate, and nothing of them kept between visits (hosted.rs).
    let runners = crate::hosted::Runners {
        jmap: crate::jmap_services::jmap_service(ctx.clone(), credentials.clone(), gate.clone()),
        jmap_fetch: crate::jmap_services::jmap_fetch_service(ctx.clone(), credentials.clone(), gate.clone()),
        jmap_pim: crate::jmap_services::jmap_pim_service(ctx, credentials, gate),
    };
    crate::hosted::hosted_window(session, services, runners)
}

/// Keep the mailbox in the browser's storage. A full store is the usual failure: say so plainly.
fn persist(bytes: &[u8]) -> Result<(), String> {
    // A look around (`?sample`, `?empty`) never overwrites the mailbox kept in this browser.
    if page::is_ephemeral(&query()) {
        return Ok(());
    }
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let store = storage().ok_or("this browser has no storage for the page (private window?)")?;
    store
        .set_item(MAILBOX_KEY, text)
        .map_err(|_| "this browser's storage for the page is full; save the mailbox as a file (File › Open & Export)".to_string())
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "download".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("eml") => "message/rfc822",
        Some("mbox") => "application/mbox",
        Some("ics") => "text/calendar",
        Some("vcf") => "text/vcard",
        Some("mailcraft" | "json") => "application/json",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}
