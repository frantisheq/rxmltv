//! Counters for the summary printed at the end, so it is clear where the time went.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use crate::t;

pub static CACHE_HITS: AtomicUsize = AtomicUsize::new(0);
pub static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);
pub static NO_DATA: AtomicUsize = AtomicUsize::new(0);
pub static FAILED: AtomicUsize = AtomicUsize::new(0);
pub static STALE: AtomicUsize = AtomicUsize::new(0);
pub static REJECTED: AtomicUsize = AtomicUsize::new(0);

pub static READ_MILLIS: AtomicUsize = AtomicUsize::new(0);
pub static EXTRACT_MILLIS: AtomicUsize = AtomicUsize::new(0);
pub static WRITE_MILLIS: AtomicUsize = AtomicUsize::new(0);

pub fn inc(counter: &AtomicUsize) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub fn add_since(counter: &AtomicUsize, since: Instant) {
    counter.fetch_add(since.elapsed().as_millis() as usize, Ordering::Relaxed);
}

fn get(counter: &AtomicUsize) -> u64 {
    counter.load(Ordering::Relaxed) as u64
}

fn seconds(counter: &AtomicUsize) -> u64 {
    (get(counter) as f64 / 1e3).round() as u64
}

/// 1234567 -> "1,234,567" (like .NET's N0).
pub fn n0(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The summary lines, as printed at the end of a run.
pub fn summary_lines() -> Vec<String> {
    let mut lines = vec![t!(
        "Files: {cache} from cache, {downloaded} downloaded, {nodata} without data, {failed} failed.",
        cache = n0(get(&CACHE_HITS)),
        downloaded = n0(get(&DOWNLOADS)),
        nodata = n0(get(&NO_DATA)),
        failed = n0(get(&FAILED))
    )];
    let stale = get(&STALE);
    if stale > 0 {
        lines.push(t!(
            "Detail files that belonged to another programme: {stale} (fetched again), {rejected} still wrong and ignored.",
            stale = n0(stale),
            rejected = n0(get(&REJECTED))
        ));
    }
    lines.push(t!(
        "Time summed over all threads: loading {read}s, extracting {extract}s, writing {write}s.",
        read = n0(seconds(&READ_MILLIS)),
        extract = n0(seconds(&EXTRACT_MILLIS)),
        write = n0(seconds(&WRITE_MILLIS))
    ));
    lines
}

pub fn print() {
    for line in summary_lines() {
        crate::progress::info(line);
    }
}

/// Files from the cache, downloaded, without data, failed. Used by the terminal UI.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn counts() -> [u64; 4] {
    [get(&CACHE_HITS), get(&DOWNLOADS), get(&NO_DATA), get(&FAILED)]
}

/// Starts the counters from zero again (the terminal UI can run several updates in one session).
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn reset() {
    for counter in [
        &CACHE_HITS, &DOWNLOADS, &NO_DATA, &FAILED, &STALE, &REJECTED, &READ_MILLIS, &EXTRACT_MILLIS, &WRITE_MILLIS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::n0;

    #[test]
    fn thousands_separators() {
        assert_eq!(n0(0), "0");
        assert_eq!(n0(999), "999");
        assert_eq!(n0(1_000), "1,000");
        assert_eq!(n0(1_234_567), "1,234,567");
    }
}
