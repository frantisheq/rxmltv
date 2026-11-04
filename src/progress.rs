//! Where the generator reports what it is doing.
//!
//! On the command line that is stderr, exactly as before. The terminal UI switches to "quiet" mode: the same
//! messages are then kept in memory and the progress in counters, so the UI can draw them inside its own
//! screen instead of having text scribble over it.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const MAX_LOG_LINES: usize = 1000;

static QUIET: AtomicBool = AtomicBool::new(false);
static DONE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static CURRENT: Mutex<String> = Mutex::new(String::new());
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// In quiet mode nothing is printed; messages go to the in-memory log instead.
pub fn set_quiet(value: bool) {
    QUIET.store(value, Ordering::Relaxed);
}

fn push(line: String) {
    let mut log = lock(&LOG);
    if log.len() >= MAX_LOG_LINES {
        log.pop_front();
    }
    log.push_back(line);
}

/// A normal message.
pub fn info(text: impl Into<String>) {
    let text = text.into();
    if quiet() {
        push(text);
    } else {
        eprintln!("{text}");
    }
}

/// A warning. On the command line it starts on a fresh line, so it never lands in the middle of the
/// `(3/120) Nova` progress line.
pub fn warn(text: impl Into<String>) {
    let text = text.into();
    if quiet() {
        push(text);
    } else {
        eprintln!("\n{text}");
    }
}

/// Forgets the previous run: progress, current channel and log.
pub fn reset() {
    DONE.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);
    lock(&CURRENT).clear();
    lock(&LOG).clear();
}

/// The number of channels that are about to be processed.
pub fn begin(total: usize) {
    DONE.store(0, Ordering::Relaxed);
    TOTAL.store(total, Ordering::Relaxed);
}

/// One more channel is finished.
pub fn step(done: usize, total: usize, name: &str) {
    DONE.store(done, Ordering::Relaxed);
    TOTAL.store(total, Ordering::Relaxed);
    if quiet() {
        let mut current = lock(&CURRENT);
        current.clear();
        current.push_str(name);
    } else {
        let line = format!("\r({done}/{total}) {name}");
        eprint!("{line:<70}");
    }
}

/// Ends the `(3/120) Nova` line.
pub fn end_steps() {
    if !quiet() {
        eprintln!();
    }
}

pub struct Snapshot {
    pub done: usize,
    pub total: usize,
    pub current: String,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        done: DONE.load(Ordering::Relaxed),
        total: TOTAL.load(Ordering::Relaxed),
        current: lock(&CURRENT).clone(),
    }
}

/// The last `max` log lines, oldest first.
pub fn log_lines(max: usize) -> Vec<String> {
    let log = lock(&LOG);
    let skip = log.len().saturating_sub(max);
    log.iter().skip(skip).cloned().collect()
}
