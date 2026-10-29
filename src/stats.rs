//! Counters for the summary printed at the end, so it is clear where the time went.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub static DOWNLOADS: AtomicU64 = AtomicU64::new(0);
pub static NO_DATA: AtomicU64 = AtomicU64::new(0);
pub static FAILED: AtomicU64 = AtomicU64::new(0);
pub static STALE: AtomicU64 = AtomicU64::new(0);
pub static REJECTED: AtomicU64 = AtomicU64::new(0);

pub static READ_NANOS: AtomicU64 = AtomicU64::new(0);
pub static EXTRACT_NANOS: AtomicU64 = AtomicU64::new(0);
pub static WRITE_NANOS: AtomicU64 = AtomicU64::new(0);

pub fn inc(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub fn add_since(counter: &AtomicU64, since: Instant) {
    counter.fetch_add(since.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

fn get(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Relaxed)
}

fn seconds(counter: &AtomicU64) -> u64 {
    (get(counter) as f64 / 1e9).round() as u64
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
    let mut lines = vec![format!(
        "Files: {} from cache, {} downloaded, {} without data, {} failed.",
        n0(get(&CACHE_HITS)),
        n0(get(&DOWNLOADS)),
        n0(get(&NO_DATA)),
        n0(get(&FAILED))
    )];
    let stale = get(&STALE);
    if stale > 0 {
        lines.push(format!(
            "Detail files that belonged to another programme: {} (fetched again), {} still wrong and ignored.",
            n0(stale),
            n0(get(&REJECTED))
        ));
    }
    lines.push(format!(
        "Time summed over all threads: loading {}s, extracting {}s, writing {}s.",
        n0(seconds(&READ_NANOS)),
        n0(seconds(&EXTRACT_NANOS)),
        n0(seconds(&WRITE_NANOS))
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
        &CACHE_HITS, &DOWNLOADS, &NO_DATA, &FAILED, &STALE, &REJECTED, &READ_NANOS, &EXTRACT_NANOS, &WRITE_NANOS,
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
