// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! The stopwatch the browser build takes its frame budget by.

use std::time::Duration;

use super::{Stopwatch, between};

#[test]
fn the_time_between_two_readings_of_a_clock_is_what_went_by_and_never_less_than_none() {
    assert_eq!(between(1_000.0, 1_250.0), Duration::from_millis(250));
    // A clock that stepped back, said nothing, or said something that is no number: no time went by.
    assert_eq!(between(1_250.0, 1_000.0), Duration::ZERO);
    assert_eq!(between(1_000.0, 0.0), Duration::ZERO);
    assert_eq!(between(f64::NAN, 1_000.0), Duration::ZERO);
    assert_eq!(between(1_000.0, f64::INFINITY), Duration::ZERO);
}

#[test]
fn a_stopwatch_counts_up_from_nothing() {
    let watch = Stopwatch::start();
    let first = watch.elapsed();
    assert!(first < Duration::from_secs(5), "{first:?}");
    std::thread::sleep(Duration::from_millis(15));
    assert!(watch.elapsed() >= first);
}
