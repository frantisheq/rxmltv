//! Downloads XML into an on-disk cache, with retries and a cap on parallel requests.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
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
    /// New cache files are written gzip-compressed (reading handles both kinds).
    compress: bool,
    http: reqwest::Client,
    gate: Semaphore,
}

impl Fetcher {
    pub fn new(cache_dir: &Path, concurrency: usize, retry_missing: bool, compress: bool) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(concurrency)
            .build()?;

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            retry_missing,
            compress,
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
        stats::add_since(&stats::READ_MILLIS, started);
        result
    }

    async fn get_core<T, F>(&self, relative_uri: &str, cache_file: &str, refresh: bool, parse: &F) -> Option<T>
    where
        F: Fn(Node<'_, '_>) -> T,
    {
        let path = self.cache_dir.join(cache_file);
        let miss_path = with_suffix(&path, ".miss");

        if refresh {
            delete_cached(&path); // throw the cached copy away and download it again
        }

        // 1. In the cache (plain or compressed): a synchronous read.
        if cache_exists(&path) {
            let cached = read_cached(&path).and_then(|bytes| parse_xml(&bytes, parse));
            if let Some(value) = cached {
                stats::inc(&stats::CACHE_HITS);
                return Some(value);
            }
            delete_cached(&path); // unreadable or corrupt cache entry: fetch it again
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
                            store(&path, &bytes, self.compress);
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
        delete_cached(&path);
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

/// The compressed twin of a cache file: "x.xml" -> "x.xml.gz".
fn gz_path(path: &Path) -> PathBuf {
    with_suffix(path, ".gz")
}

fn cache_exists(path: &Path) -> bool {
    path.exists() || gz_path(path).exists()
}

fn delete_cached(path: &Path) {
    try_delete(path);
    try_delete(&gz_path(path));
}

/// The bytes of a cache entry, whether it is stored plain or compressed.
fn read_cached(path: &Path) -> Option<Vec<u8>> {
    if let Ok(bytes) = fs::read(path) {
        return Some(bytes);
    }
    let packed = fs::read(gz_path(path)).ok()?;
    let mut plain = Vec::new();
    GzDecoder::new(&packed[..]).read_to_end(&mut plain).ok()?;
    Some(plain)
}

fn gzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// Writes the cache entry (compressed or plain, and removes the other kind if there is one). There is no
/// `await` between creating the temporary file and renaming it, so cancelling the program can never leave
/// a half-written entry behind.
fn store(path: &Path, bytes: &[u8], compress: bool) {
    let (target, other) = if compress { (gz_path(path), path.to_path_buf()) } else { (path.to_path_buf(), gz_path(path)) };
    let tmp = with_suffix(&target, ".tmp");

    let result = (|| -> io::Result<()> {
        let data = if compress { gzip(bytes)? } else { bytes.to_vec() };
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &target)
    })();

    match result {
        Ok(()) => try_delete(&other),
        Err(e) => {
            try_delete(&tmp);
            crate::progress::warn(format!("Warning: could not write {}: {e}", target.display()));
        }
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
        } else if let Some(stem) = file_name.strip_suffix(".gz").unwrap_or(file_name).strip_suffix(".xml") {
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

/// What a purge or a compression did.
pub struct CacheReport {
    pub files: usize,
    /// Bytes deleted (purge) or saved (compression).
    pub bytes: u64,
}

/// Only files the cache itself makes are ever touched, so a wrong cache folder setting cannot cost anything else.
fn is_cache_file(name: &str) -> bool {
    [".xml", ".xml.gz", ".miss", ".tmp"].iter().any(|suffix| name.ends_with(suffix))
}

/// Deletes every cached file (data, "no data" markers and leftovers), including the channel list.
pub fn purge_cache(cache_dir: &Path) -> io::Result<CacheReport> {
    let mut report = CacheReport { files: 0, bytes: 0 };
    let entries = match fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !is_cache_file(name) || !path.is_file() {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if fs::remove_file(&path).is_ok() {
            report.files += 1;
            report.bytes += size;
        }
    }
    Ok(report)
}

/// Compresses the cache files that are still plain XML ("x.xml" becomes "x.xml.gz").
pub fn compress_existing(cache_dir: &Path) -> CacheReport {
    let mut report = CacheReport { files: 0, bytes: 0 };
    let Ok(entries) = fs::read_dir(cache_dir) else { return report };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_xml = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".xml"));
        if !is_xml || !path.is_file() {
            continue;
        }
        let Ok(plain) = fs::read(&path) else { continue };
        let Ok(packed) = gzip(&plain) else { continue };

        let target = gz_path(&path);
        let tmp = with_suffix(&target, ".tmp");
        if fs::write(&tmp, &packed).and_then(|_| fs::rename(&tmp, &target)).is_ok() {
            try_delete(&path);
            report.files += 1;
            report.bytes += (plain.len() as u64).saturating_sub(packed.len() as u64);
        } else {
            try_delete(&tmp);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rxmltv-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn compressed_entries_are_read_back() {
        let dir = temp_dir("fetch-gz");
        let path = dir.join("a-20261029.xml");
        store(&path, b"<a>\xC4\x8D</a>", true);
        assert!(!path.exists());
        assert!(gz_path(&path).exists());
        assert_eq!(read_cached(&path).as_deref(), Some(&b"<a>\xC4\x8D</a>"[..]));

        // switching compression off replaces the compressed file
        store(&path, b"<a/>", false);
        assert!(path.exists() && !gz_path(&path).exists());
        assert_eq!(read_cached(&path).as_deref(), Some(&b"<a/>"[..]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_files_are_compressed() {
        let dir = temp_dir("fetch-compress");
        let big = "<x>0123456789</x>".repeat(200);
        fs::write(dir.join("a-20261029.xml"), &big).unwrap();
        fs::write(dir.join("notes.txt"), "keep").unwrap();

        let report = compress_existing(&dir);
        assert_eq!(report.files, 1);
        assert!(report.bytes > 0);
        assert!(!dir.join("a-20261029.xml").exists());
        assert_eq!(read_cached(&dir.join("a-20261029.xml")).unwrap(), big.as_bytes());
        assert!(dir.join("notes.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn purging_removes_only_cache_files() {
        let dir = temp_dir("fetch-purge");
        for name in ["channels.xml", "a-20261029.xml.gz", "b-20261029.xml.miss", "c.xml.tmp", "precious.doc"] {
            fs::write(dir.join(name), "12345").unwrap();
        }
        let report = purge_cache(&dir).unwrap();
        assert_eq!(report.files, 4);
        assert_eq!(report.bytes, 20);
        assert!(dir.join("precious.doc").exists());
        assert!(purge_cache(&dir.join("missing")).unwrap().files == 0); // no folder: nothing to do
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compressed_files_are_cleaned_by_date_too() {
        let dir = temp_dir("fetch-clean");
        fs::write(dir.join("a-20200101.xml.gz"), "x").unwrap();
        fs::write(dir.join("a-20990101.xml.gz"), "x").unwrap();
        clean_cache(&dir, NaiveDate::from_ymd_opt(2026, 10, 29).unwrap());
        assert!(!dir.join("a-20200101.xml.gz").exists());
        assert!(dir.join("a-20990101.xml.gz").exists());
        let _ = fs::remove_dir_all(&dir);
    }

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
