// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A stopwatch for the code the browser build runs too.
//!
//! `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not implemented on this platform"), and the
//! first frame of the browser build died of it: `poll_sync` takes the time at every frame to see how much of the
//! frame it may spend (#991), and upstream's 0.12.0 did it with `Instant`. Here the desktop keeps its `Instant`, and
//! the browser reads its own clock ([`crate::now_ms`]), which can say how long a piece of work took, to the
//! millisecond, though not that it never goes back (a step back is no time gone by). The wasm pass of
//! `.github/workflows/neoffice-ci.yml` refuses a call of `Instant::now()` in code the browser runs
//! (`.github/clippy-wasm/clippy.toml`).

use std::time::Duration;

/// How long it has been since [`Stopwatch::start`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
    /// The browser's clock when it began, in milliseconds.
    #[cfg(target_arch = "wasm32")]
    started: f64,
}

impl Stopwatch {
    pub(crate) fn start() -> Stopwatch {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Stopwatch { started: std::time::Instant::now() }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Stopwatch { started: crate::now_ms() }
        }
    }

    pub(crate) fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            between(self.started, crate::now_ms())
        }
    }
}

/// The time from `from` to `to`, both in milliseconds of a clock: none when the clock stepped back or said nothing.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn between(from: f64, to: f64) -> Duration {
    let ms = to - from;
    if ms.is_finite() && ms > 0.0 { Duration::from_secs_f64(ms / 1000.0) } else { Duration::ZERO }
}

#[cfg(test)]
#[path = "stopwatch_tests.rs"]
mod tests;
