//! Downloads XML into an on-disk cache, with retries and a cap on parallel requests.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use reqwest::StatusCode;
use roxmltree::Node;
use tokio::sync::Semaphore;

use crate::stats;
use crate::util::{try_delete, with_suffix};

const BASE_URI: &str = "http://programandroid.365dni.cz/android/";
const USER_AGENT: &str =
    "SMSTVP/1.7.3 (242;cs_CZ; ID/ef284441-c1cd-4f9e-8e30-f5d8b1ac170c HW/Redmi Note 7 Android/10 (QKQ1.190910.002))";
const MAX_ATTEMPTS: u32 = 3;

/// How long an item the server had no data for is not asked for again.
pub const MISS_TTL: Duration = Duration::from_secs(12 * 60 * 60);

pub struct Fetcher {
    cache_dir: PathBuf,
    retry_missing: bool,
    http: reqwest::Client,
    gate: Semaphore,
}

impl Fetcher {
    pub fn new(cache_dir: &Path, concurrency: usize, retry_missing: bool) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(concurrency)
            .build()?;

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            retry_missing,
            http,
            gate: Semaphore::new(concurrency),
        })
    }

    /// Loads the XML (from the cache or the network) and hands its root element to `parse`.
    /// Returns what `parse` returned, or None when there is no data.
    ///
    /// The parsed document never leaves this function (`parse` copies out what it needs), so
    /// callers can hold the result across `await` points without keeping any XML tree alive.
    pub async fn get<T, F>(&self, relative_uri: &str, cache_file: &str, refresh: bool, parse: F) -> Option<T>
    where
        F: Fn(Node<'_, '_>) -> T,
    {
        let started = Instant::now();
        let result = self.get_core(relative_uri, cache_file, refresh, &parse).await;
        stats::add_since(&stats::READ_NANOS, started);
        result
    }

    async fn get_core<T, F>(&self, relative_uri: &str, cache_file: &str, refresh: bool, parse: &F) -> Option<T>
    where
        F: Fn(Node<'_, '_>) -> T,
    {
        let path = self.cache_dir.join(cache_file);
        let miss_path = with_suffix(&path, ".miss");

        if refresh {
            try_delete(&path); // throw the cached copy away and download it again
        }

        // 1. In the cache: a plain synchronous read.
        if path.exists() {
            let cached = fs::read(&path).ok().and_then(|bytes| parse_xml(&bytes, parse));
            if let Some(value) = cached {
                stats::inc(&stats::CACHE_HITS);
                return Some(value);
            }
            try_delete(&path); // unreadable or corrupt cache entry: fetch it again
        }

        // 2. The server had no data for this one recently, so don't ask again on every run.
        if !refresh && !self.retry_missing && is_fresh(&miss_path) {
            stats::inc(&stats::NO_DATA);
            return None;
        }

        // 3. Download.
        let uri = format!("{BASE_URI}{relative_uri}");
        for attempt in 1..=MAX_ATTEMPTS {
            match self.download(&uri).await {
                Ok(bytes) => {
                    return match parse_xml(&bytes, parse) {
                        Some(value) => {
                            store(&path, &bytes);
                            stats::inc(&stats::DOWNLOADS);
                            try_delete(&miss_path);
                            Some(value)
                        }
                        // The server answered, but not with XML.
                        None => no_data(&miss_path),
                    };
                }
                Err(e) if matches!(e.status(), Some(StatusCode::NOT_FOUND | StatusCode::GONE)) => {
                    return no_data(&miss_path); // a definite "no": retrying will not change it
                }
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        stats::inc(&stats::FAILED);
                        crate::progress::warn(format!("Warning: giving up on {cache_file}: {e}"));
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                }
            }
        }

        None
    }

    /// The file is for another programme: drop it and don't ask again for a while.
    pub fn reject(&self, cache_file: &str) {
        let path = self.cache_dir.join(cache_file);
        try_delete(&path);
        let _ = fs::write(with_suffix(&path, ".miss"), "");
    }

    async fn download(&self, uri: &str) -> Result<Vec<u8>, reqwest::Error> {
        let _permit = self.gate.acquire().await.expect("the semaphore is never closed");
        let response = self.http.get(uri).send().await?.error_for_status()?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }
}

fn no_data<T>(miss_path: &Path) -> Option<T> {
    stats::inc(&stats::NO_DATA);
    let _ = fs::write(miss_path, "");
    None
}

fn is_fresh(miss_path: &Path) -> bool {
    fs::metadata(miss_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < MISS_TTL)
}

/// Writes the cache entry. There is no `await` between creating the temporary file and renaming it,
/// so cancelling the program can never leave a half-written entry behind.
fn store(path: &Path, bytes: &[u8]) {
    let tmp = with_suffix(path, ".tmp");
    let result = fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, path));
    if let Err(e) = result {
        try_delete(&tmp);
        crate::progress::warn(format!("Warning: could not write {}: {e}", path.display()));
    }
}

fn parse_xml<T, F>(bytes: &[u8], parse: &F) -> Option<T>
where
    F: Fn(Node<'_, '_>) -> T,
{
    let text = decode(bytes)?;

    let mut options = roxmltree::ParsingOptions::default();
    options.allow_dtd = true;
    let doc = roxmltree::Document::parse_with_options(&text, options).ok()?;

    Some(parse(doc.root_element()))
}

/// UTF-8 normally; falls back to the encoding named in the XML declaration.
fn decode(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.strip_prefix(&[0xEFu8, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_owned());
    }

    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    let label = declared_encoding(&head)?;
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let (text, _, _) = encoding.decode(bytes);
    Some(text.into_owned())
}

fn declared_encoding(head: &str) -> Option<String> {
    let rest = head.split("encoding=").nth(1)?;
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    rest[1..].split(quote).next().map(str::to_string)
}

/// Deletes temporary files, expired "no data" markers and cached files older than today.
pub fn clean_cache(cache_dir: &Path, today: NaiveDate) {
    let Ok(entries) = fs::read_dir(cache_dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else { continue };

        if file_name.ends_with(".tmp") {
            try_delete(&path);
        } else if file_name.ends_with(".miss") {
            let expired = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= MISS_TTL);
            if expired {
                try_delete(&path);
            }
        } else if let Some(stem) = file_name.strip_suffix(".xml") {
            // File names look like "<id>-yyyyMMdd.xml" or "<id>-yyyyMMddHHmmss.xml".
            let Some(dash) = stem.rfind('-') else { continue };
            let Some(date_text) = stem.get(dash + 1..dash + 9) else { continue };
            if !date_text.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(file_date) = NaiveDate::parse_from_str(date_text, "%Y%m%d") {
                if file_date < today {
                    try_delete(&path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_with_bom_is_decoded() {
        assert_eq!(decode(b"\xEF\xBB\xBF<a>\xC4\x8D</a>").as_deref(), Some("<a>č</a>"));
    }

    #[test]
    fn declared_encoding_is_honoured() {
        // 0xE8 is "č" in windows-1250.
        let bytes = b"<?xml version=\"1.0\" encoding=\"windows-1250\"?><a>\xE8</a>";
        assert_eq!(
            decode(bytes).as_deref(),
            Some("<?xml version=\"1.0\" encoding=\"windows-1250\"?><a>č</a>")
        );
    }

    #[test]
    fn garbage_without_declaration_is_rejected() {
        assert_eq!(decode(b"<a>\xFF\xFE</a>"), None);
    }

    #[test]
    fn parse_hands_over_the_root_element() {
        let name = parse_xml(b"<root><x/></root>", &|root: Node| root.tag_name().name().to_string());
        assert_eq!(name.as_deref(), Some("root"));
        assert_eq!(parse_xml(b"not xml", &|root: Node| root.tag_name().name().to_string()), None);
    }
}
