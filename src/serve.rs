//! A small HTTP server that shares the generated XMLTV file.
//!
//! It serves exactly one file (and a tiny index page), answers GET and HEAD, supports conditional requests
//! (ETag, Last-Modified) so clients that poll only download a changed guide, and closes the connection after
//! every response. It never looks at the file system for anything the request names, so there is no way to ask
//! it for any other file.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::t;

/// One answered request, for the log.
pub struct RequestLog {
    pub peer: String,
    /// "GET /xmltv.xml"
    pub request: String,
    pub status: u16,
    pub bytes: u64,
}

pub type Logger = Arc<dyn Fn(RequestLog) + Send + Sync>;

const MAX_HEAD: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 64;

// ---------- starting ----------

pub async fn bind(address: &str, port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((address.trim(), port)).await.map_err(|e| {
        t!("Cannot listen on {address}:{port}: {error}", address = address.trim(), port = port, error = e)
    })
}

/// Accepts connections until the future is dropped.
pub async fn run(listener: TcpListener, file: PathBuf, log: Logger) {
    let limit = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&limit).acquire_owned().await else { return };
        let file = file.clone();
        let log = Arc::clone(&log);
        tokio::spawn(async move {
            let _permit = permit;
            handle(stream, peer, &file, &log).await;
        });
    }
}

/// The addresses to give to people: for "listen on everything" that is localhost and this computer's address.
pub fn urls(addr: SocketAddr, file_name: &str) -> Vec<String> {
    let path = format!("/{}", encode_path(file_name));
    let port = addr.port();
    let host = |ip: IpAddr| match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };

    if addr.ip().is_unspecified() {
        let mut list = vec![format!("http://localhost:{port}{path}")];
        if let Some(ip) = local_ip() {
            list.push(format!("http://{}:{port}{path}", host(ip)));
        }
        list
    } else {
        vec![format!("http://{}:{port}{path}", host(addr.ip()))]
    }
}

/// This computer's address on the network it reaches the internet through (no packet is sent).
fn local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip)
    }
}

// ---------- requests ----------

pub struct Request {
    pub method: String,
    pub path: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

pub fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?;
    if !first.next()?.starts_with("HTTP/") {
        return None;
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    if !path.starts_with('/') {
        return None;
    }

    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Some(Request { method, path, headers })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let value = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(value) = value {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Everything except letters, digits and `-._~` becomes %XX.
fn encode_path(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Index,
    File,
    NotFound,
    MethodNotAllowed,
}

pub fn route(method: &str, path: &str, file_name: &str) -> Route {
    if method != "GET" && method != "HEAD" {
        return Route::MethodNotAllowed;
    }
    if path == "/" || path == "/index.html" {
        return Route::Index;
    }
    if percent_decode(path.trim_start_matches('/')) == file_name {
        return Route::File;
    }
    Route::NotFound
}

// ---------- the file's headers ----------

pub fn http_date(secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(secs, 0)
        .map(|d| d.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        .unwrap_or_default()
}

fn parse_http_date(text: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(text.trim(), "%a, %d %b %Y %H:%M:%S GMT").ok().map(|n| n.and_utc().timestamp())
}

pub fn etag(size: u64, modified: i64) -> String {
    format!("\"{size:x}-{modified:x}\"")
}

/// Does the client already have this version of the file?
pub fn not_modified(request: &Request, etag: &str, modified: i64) -> bool {
    if let Some(wanted) = request.header("if-none-match") {
        return wanted.split(',').map(str::trim).any(|t| t == "*" || t.trim_start_matches("W/") == etag);
    }
    if let Some(since) = request.header("if-modified-since").and_then(parse_http_date) {
        return modified <= since;
    }
    false
}

fn content_type(file_name: &str) -> &'static str {
    if file_name.ends_with(".gz") {
        "application/gzip"
    } else {
        "application/xml; charset=utf-8"
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn index_page(file_name: &str, info: Option<&FileInfo>) -> String {
    let link = encode_path(file_name);
    let name = html_escape(file_name);
    let details = match info {
        Some(i) => format!(
            "{} &middot; updated {}",
            crate::upload::human_size(i.size),
            http_date(i.modified)
        ),
        None => "not generated yet".to_string(),
    };
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>rxmltv</title></head>\n<body>\n<h1>rxmltv</h1>\n<p><a href=\"/{link}\">{name}</a> ({details})</p>\n</body></html>\n"
    )
}

struct FileInfo {
    size: u64,
    modified: i64,
}

async fn file_info(path: &Path) -> Option<FileInfo> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(FileInfo { size: meta.len(), modified })
}

// ---------- one connection ----------

struct Served {
    request: String,
    status: u16,
    bytes: u64,
}

async fn handle(mut stream: TcpStream, peer: SocketAddr, file: &Path, log: &Logger) {
    let served = tokio::time::timeout(Duration::from_secs(120), respond(&mut stream, file)).await;
    if let Ok(Some(served)) = served {
        log(RequestLog { peer: peer.ip().to_string(), request: served.request, status: served.status, bytes: served.bytes });
    }
    let _ = stream.shutdown().await;
}

/// The request head (up to the empty line), or None when the client is too slow, too chatty or gone.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let reading = async {
        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            let n = stream.read(&mut buf).await.ok()?;
            if n == 0 {
                return None;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                return Some(String::from_utf8_lossy(&data[..end]).to_string());
            }
            if data.len() > MAX_HEAD {
                return None;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), reading).await.ok().flatten()
}

fn head_text(status: u16, reason: &str, headers: &[(&str, String)]) -> String {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nServer: rxmltv\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    head
}

/// Sends a short response (an error, the index page, a 304). Returns the body bytes sent.
async fn send_small(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &str,
    extra: &[(&str, String)],
    head_only: bool,
) -> u64 {
    let mut headers: Vec<(&str, String)> =
        vec![("Content-Type", content_type.to_string()), ("Content-Length", body.len().to_string())];
    headers.extend(extra.iter().map(|(n, v)| (*n, v.clone())));
    let _ = stream.write_all(head_text(status, reason, &headers).as_bytes()).await;
    if head_only || status == 304 {
        return 0;
    }
    let _ = stream.write_all(body.as_bytes()).await;
    body.len() as u64
}

async fn respond(stream: &mut TcpStream, file: &Path) -> Option<Served> {
    let head = read_head(stream).await?;
    let Some(request) = parse_request(&head) else {
        let bytes = send_small(stream, 400, "Bad Request", "text/plain", "Bad request\n", &[], false).await;
        return Some(Served { request: "?".to_string(), status: 400, bytes });
    };

    let label = format!("{} {}", request.method, request.path);
    let head_only = request.method == "HEAD";
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "xmltv.xml".to_string());
    let info = file_info(file).await;

    let (status, bytes) = match route(&request.method, &request.path, &name) {
        Route::MethodNotAllowed => {
            let b = send_small(stream, 405, "Method Not Allowed", "text/plain", "Method not allowed\n", &[("Allow", "GET, HEAD".to_string())], false).await;
            (405, b)
        }
        Route::NotFound => {
            let b = send_small(stream, 404, "Not Found", "text/plain", "Not found\n", &[], head_only).await;
            (404, b)
        }
        Route::Index => {
            let page = index_page(&name, info.as_ref());
            let b = send_small(stream, 200, "OK", "text/html; charset=utf-8", &page, &[("Cache-Control", "no-cache".to_string())], head_only).await;
            (200, b)
        }
        Route::File => match info {
            None => {
                let b = send_small(stream, 503, "Service Unavailable", "text/plain", "The guide has not been generated yet\n", &[("Retry-After", "60".to_string())], head_only).await;
                (503, b)
            }
            Some(info) => {
                let tag = etag(info.size, info.modified);
                let validators = vec![
                    ("ETag", tag.clone()),
                    ("Last-Modified", http_date(info.modified)),
                    ("Cache-Control", "no-cache".to_string()),
                ];
                if not_modified(&request, &tag, info.modified) {
                    send_small(stream, 304, "Not Modified", content_type(&name), "", &validators, true).await;
                    (304, 0)
                } else {
                    match tokio::fs::File::open(file).await {
                        Err(_) => {
                            let b = send_small(stream, 503, "Service Unavailable", "text/plain", "The guide is not available\n", &[("Retry-After", "60".to_string())], head_only).await;
                            (503, b)
                        }
                        Ok(mut opened) => {
                            let mut headers = vec![
                                ("Content-Type", content_type(&name).to_string()),
                                ("Content-Length", info.size.to_string()),
                            ];
                            headers.extend(validators);
                            let _ = stream.write_all(head_text(200, "OK", &headers).as_bytes()).await;
                            let sent = if head_only { 0 } else { tokio::io::copy(&mut opened, stream).await.unwrap_or(0) };
                            (200, sent)
                        }
                    }
                }
            }
        },
    };

    Some(Served { request: label, status, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> Request {
        parse_request(text).unwrap()
    }

    #[test]
    fn requests_are_parsed() {
        let r = request("GET /xmltv.xml?x=1 HTTP/1.1\r\nHost: example\r\nIf-None-Match: \"a-b\"\r\n");
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/xmltv.xml");
        assert_eq!(r.header("if-none-match"), Some("\"a-b\""));
        assert!(parse_request("nonsense").is_none());
        assert!(parse_request("GET relative HTTP/1.1").is_none());
        assert!(parse_request("GET / SPDY/3").is_none());
    }

    #[test]
    fn only_the_guide_is_served() {
        assert_eq!(route("GET", "/", "epg.xml.gz"), Route::Index);
        assert_eq!(route("HEAD", "/epg.xml.gz", "epg.xml.gz"), Route::File);
        assert_eq!(route("GET", "/my%20guide.xml", "my guide.xml"), Route::File);
        assert_eq!(route("GET", "/other.xml", "epg.xml.gz"), Route::NotFound);
        assert_eq!(route("GET", "/../etc/passwd", "epg.xml.gz"), Route::NotFound);
        assert_eq!(route("POST", "/epg.xml.gz", "epg.xml.gz"), Route::MethodNotAllowed);
    }

    #[test]
    fn dates_and_tags() {
        assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(etag(255, 16), "\"ff-10\"");
    }

    #[test]
    fn clients_with_the_current_version_get_304() {
        let tag = etag(100, 1_000);
        let same = request(&format!("GET / HTTP/1.1\r\nIf-None-Match: {tag}\r\n"));
        assert!(not_modified(&same, &tag, 1_000));
        let weak = request(&format!("GET / HTTP/1.1\r\nIf-None-Match: W/{tag}, \"x\"\r\n"));
        assert!(not_modified(&weak, &tag, 1_000));
        let other = request("GET / HTTP/1.1\r\nIf-None-Match: \"old\"\r\n");
        assert!(!not_modified(&other, &tag, 1_000));

        let date = http_date(1_000);
        let fresh = request(&format!("GET / HTTP/1.1\r\nIf-Modified-Since: {date}\r\n"));
        assert!(not_modified(&fresh, &tag, 1_000));
        assert!(!not_modified(&fresh, &tag, 2_000)); // the file changed since
        assert!(!not_modified(&request("GET / HTTP/1.1\r\n"), &tag, 1_000));
    }

    #[test]
    fn addresses_are_given_for_people() {
        let any: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert_eq!(urls(any, "my guide.xml")[0], "http://localhost:8080/my%20guide.xml");
        let one: SocketAddr = "192.168.1.5:80".parse().unwrap();
        assert_eq!(urls(one, "epg.xml"), vec!["http://192.168.1.5:80/epg.xml".to_string()]);
        let v6: SocketAddr = "[::1]:9000".parse().unwrap();
        assert_eq!(urls(v6, "epg.xml"), vec!["http://[::1]:9000/epg.xml".to_string()]);
    }

    #[test]
    fn percent_signs_decode() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%C4%8D"), "č");
    }

    async fn fetch(addr: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await.unwrap();
        String::from_utf8_lossy(&answer).to_string()
    }

    #[tokio::test]
    async fn the_server_serves_the_file() {
        let dir = std::env::temp_dir().join(format!("rxmltv-test-{}-serve", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("epg.xml");
        std::fs::write(&file, "<tv></tv>").unwrap();

        let listener = bind("127.0.0.1", 0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(u16, u64)>::new()));
        let seen2 = Arc::clone(&seen);
        let log: Logger = Arc::new(move |entry| seen2.lock().unwrap().push((entry.status, entry.bytes)));
        let server = tokio::spawn(run(listener, file.clone(), log));

        let ok = fetch(addr, "GET /epg.xml HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(ok.starts_with("HTTP/1.1 200 OK"));
        assert!(ok.contains("Content-Length: 9"));
        assert!(ok.contains("application/xml"));
        assert!(ok.ends_with("<tv></tv>"));

        let head = fetch(addr, "HEAD /epg.xml HTTP/1.1\r\n\r\n").await;
        assert!(head.starts_with("HTTP/1.1 200 OK") && !head.contains("<tv>"));

        let tag = ok.lines().find_map(|l| l.strip_prefix("ETag: ")).unwrap().to_string();
        let cached = fetch(addr, &format!("GET /epg.xml HTTP/1.1\r\nIf-None-Match: {tag}\r\n\r\n")).await;
        assert!(cached.starts_with("HTTP/1.1 304"));

        assert!(fetch(addr, "GET /nope HTTP/1.1\r\n\r\n").await.starts_with("HTTP/1.1 404"));
        assert!(fetch(addr, "POST /epg.xml HTTP/1.1\r\n\r\n").await.starts_with("HTTP/1.1 405"));
        assert!(fetch(addr, "GET / HTTP/1.1\r\n\r\n").await.contains("<a href=\"/epg.xml\">"));
        assert!(fetch(addr, "garbage\r\n\r\n").await.starts_with("HTTP/1.1 400"));

        std::fs::remove_file(&file).unwrap();
        assert!(fetch(addr, "GET /epg.xml HTTP/1.1\r\n\r\n").await.starts_with("HTTP/1.1 503"));

        server.abort();
        let log = seen.lock().unwrap();
        assert!(log.contains(&(200, 9)));
        assert!(log.iter().any(|(status, _)| *status == 304));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
