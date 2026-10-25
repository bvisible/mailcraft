// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! Whether the browser's WebGPU gives an adapter in time, asked before the window starts.
//!
//! eframe asks the browser for a WebGPU adapter before it draws anything (`navigator.gpu.requestAdapter()`,
//! through wgpu's WebGPU detection), and waits for the answer however long it takes. A browser whose GPU side is
//! slow to answer, or never answers (a Chrome without a screen, its caches just emptied: every first load stayed
//! on "Loading NeoMail…"), kept the page waiting with it. Asked here first, with a limit: an adapter in time and
//! the window starts as before (WebGPU); no WebGPU, no adapter or no answer in time and it starts on WebGL2 at once,
//! as `?webgl` asks.

use wasm_bindgen::{JsCast as _, JsValue};

/// How long the browser's WebGPU may take to give an adapter, in milliseconds.
pub const WAIT_MS: i32 = 3000;

/// `true` when `navigator.gpu.requestAdapter()` gives an adapter within [`WAIT_MS`]; `false` when the browser has
/// no WebGPU, gives none, refuses, or doesn't answer in time.
pub async fn answers() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let gpu = js_sys::Reflect::get(&window.navigator(), &JsValue::from_str("gpu")).unwrap_or(JsValue::UNDEFINED);
    if gpu.is_undefined() || gpu.is_null() {
        return false;
    }
    let Some(request) = js_sys::Reflect::get(&gpu, &JsValue::from_str("requestAdapter")).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return false;
    };
    let Some(asked) = request.call0(&gpu).ok().and_then(|p| p.dyn_into::<js_sys::Promise>().ok()) else {
        return false;
    };
    // No answer in time reads as no adapter (the timer resolves with nothing).
    let timer = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, WAIT_MS);
    });
    let first = js_sys::Promise::race(&js_sys::Array::of2(&asked, &timer));
    match wasm_bindgen_futures::JsFuture::from(first).await {
        Ok(adapter) => !adapter.is_undefined() && !adapter.is_null(),
        Err(_) => false,
    }
}
