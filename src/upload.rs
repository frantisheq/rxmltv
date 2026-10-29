//! Uploading the finished guide (FTP, FTPS, SFTP, WebDAV) and running a command after an update.
//!
//! The transfer itself is done by the `curl` program, which every platform rxmltv runs on has or can install,
//! and which already speaks all of these protocols (including SSH keys and TLS). That keeps rxmltv free of
//! TLS and SSH code. Everything secret (user name, password, key passphrase) is handed to curl on its
//! standard input, never on the command line, so it does not show up in the process list.

#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Ftp,
    Ftps,
    Sftp,
    WebDav,
    WebDavs,
}

impl Protocol {
    pub const ALL: [Protocol; 5] = [Protocol::Ftp, Protocol::Ftps, Protocol::Sftp, Protocol::WebDav, Protocol::WebDavs];

    /// The name used in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Sftp => "sftp",
            Protocol::WebDav => "webdav",
            Protocol::WebDavs => "webdavs",
        }
    }

    pub fn parse(text: &str) -> Option<Protocol> {
        let t = text.trim().to_ascii_lowercase();
        Protocol::ALL.into_iter().find(|p| p.name() == t)
    }

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS (FTP with TLS)",
            Protocol::Sftp => "SFTP (over SSH)",
            Protocol::WebDav => "WebDAV (http)",
            Protocol::WebDavs => "WebDAV (https)",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::Sftp => 22,
            Protocol::WebDav => 80,
            Protocol::WebDavs => 443,
        }
    }

    /// The next (or, with a negative delta, previous) protocol, wrapping around.
    pub fn step(self, delta: i32) -> Protocol {
        let n = Protocol::ALL.len() as i32;
        let at = Protocol::ALL.iter().position(|p| *p == self).unwrap_or(0) as i32;
        Protocol::ALL[(at + delta).rem_euclid(n) as usize]
    }

    fn scheme(self) -> &'static str {
        match self {
            Protocol::Ftp | Protocol::Ftps => "ftp",
            Protocol::Sftp => "sftp",
            Protocol::WebDav => "http",
            Protocol::WebDavs => "https",
        }
    }
}

#[derive(Clone, Debug)]
pub struct UploadOptions {
    /// Upload automatically after every update (the Upload tab can always upload by hand).
    pub enabled: bool,
    pub protocol: Protocol,
    pub host: String,
    /// None: the protocol's usual port.
    pub port: Option<u16>,
    pub username: String,
    /// Empty: the RXMLTV_UPLOAD_PASSWORD environment variable, if set.
    pub password: String,
    /// The folder on the server. A leading / means an absolute path (FTP, SFTP); otherwise it is relative to
    /// the folder you land in after logging in.
    pub folder: String,
    /// The file name on the server. Empty: the name of the output file.
    pub remote_name: String,
    /// SSH private key for SFTP.
    pub key_file: String,
    pub key_passphrase: String,
    /// Do not verify the TLS certificate (FTPS, https) or the SSH host key (SFTP).
    pub insecure: bool,
}

impl Default for UploadOptions {
    fn default() -> Self {
        UploadOptions {
            enabled: false,
            protocol: Protocol::Ftp,
            host: String::new(),
            port: None,
            username: String::new(),
            password: String::new(),
            folder: String::new(),
            remote_name: String::new(),
            key_file: String::new(),
            key_passphrase: String::new(),
            insecure: false,
        }
    }
}

/// Everything except letters, digits and `-._~` becomes %XX.
fn encode_segment(text: &str) -> String {
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

/// A value for curl's config file: in quotes, with backslash and quote escaped.
fn cfg_quote(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn human_size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} kB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

impl UploadOptions {
    pub fn effective_password(&self) -> String {
        if self.password.is_empty() {
            std::env::var("RXMLTV_UPLOAD_PASSWORD").unwrap_or_default()
        } else {
            self.password.clone()
        }
    }

    /// The name the file gets on the server.
    pub fn target_name(&self, output: &Path) -> String {
        let own = self.remote_name.trim();
        if !own.is_empty() {
            return own.to_string();
        }
        output.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "xmltv.xml".to_string())
    }

    /// The full URL of the uploaded file (never contains the user name or password).
    pub fn remote_url(&self, file_name: &str) -> Result<String, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err("No server host is set.".to_string());
        }
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
        let port = match self.port {
            Some(port) => format!(":{port}"),
            None => String::new(),
        };

        let folder = self.folder.trim();
        let absolute = folder.starts_with('/');
        let mut path = String::new();
        match (self.protocol, absolute) {
            (Protocol::Sftp, false) => path.push_str("/~"), // curl: relative to the home folder
            (Protocol::Ftp | Protocol::Ftps, true) => path.push('/'), // curl: "//" starts an absolute path
            _ => {}
        }
        for segment in folder.split('/').filter(|s| !s.is_empty() && *s != ".") {
            path.push('/');
            path.push_str(&encode_segment(segment));
        }
        path.push('/');
        path.push_str(&encode_segment(file_name));

        Ok(format!("{}://{}{}{}", self.protocol.scheme(), host, port, path))
    }

    /// What curl reads on its standard input.
    pub fn curl_config(&self, url: &str, file: &Path) -> String {
        let mut c = String::new();
        c.push_str(&format!("url = {}\n", cfg_quote(url)));
        c.push_str(&format!("upload-file = {}\n", cfg_quote(&file.to_string_lossy())));
        c.push_str("progress-bar\nshow-error\nfail\nconnect-timeout = 30\n");

        let user = self.username.trim();
        if !user.is_empty() {
            c.push_str(&format!("user = {}\n", cfg_quote(&format!("{user}:{}", self.effective_password()))));
        }
        match self.protocol {
            Protocol::Ftp => c.push_str("ftp-create-dirs\n"),
            Protocol::Ftps => c.push_str("ftp-create-dirs\nssl-reqd\n"),
            Protocol::Sftp => {
                c.push_str("ftp-create-dirs\n");
                let key = self.key_file.trim();
                if !key.is_empty() {
                    c.push_str(&format!("key = {}\n", cfg_quote(key)));
                    let public = format!("{key}.pub");
                    if Path::new(&public).is_file() {
                        c.push_str(&format!("pubkey = {}\n", cfg_quote(&public)));
                    }
                    if !self.key_passphrase.is_empty() {
                        c.push_str(&format!("pass = {}\n", cfg_quote(&self.key_passphrase)));
                    }
                }
            }
            Protocol::WebDav | Protocol::WebDavs => {}
        }
        if self.insecure {
            c.push_str("insecure\n");
        }
        c
    }
}

pub enum Event {
    Log(String),
    /// Percent, 0 to 100.
    Progress(f64),
}

/// "######## 45.2%" gives 45.2.
fn parse_percent(text: &str) -> Option<f64> {
    let body = text.trim().strip_suffix('%')?;
    let number = body.rsplit(|c: char| c.is_whitespace() || c == '#').next()?;
    number.parse::<f64>().ok().map(|p| p.clamp(0.0, 100.0))
}

fn handle_chunk(pending: &mut Vec<u8>, on_event: &mut impl FnMut(Event), errors: &mut Vec<String>) {
    let text = String::from_utf8_lossy(pending).trim().to_string();
    pending.clear();
    if text.is_empty() || text.chars().all(|c| matches!(c, '#' | '=' | '-' | 'O' | ' ')) {
        return; // empty, or a bar with no number yet
    }
    match parse_percent(&text) {
        Some(percent) => on_event(Event::Progress(percent)),
        None => {
            errors.push(text.clone());
            on_event(Event::Log(text));
        }
    }
}

/// Uploads `file`; progress and messages go to `on_event`. Dropping the future stops curl.
pub async fn upload(o: &UploadOptions, file: &Path, mut on_event: impl FnMut(Event)) -> Result<(), String> {
    let size = std::fs::metadata(file).map_err(|e| format!("Cannot read {}: {e}", file.display()))?.len();
    let url = o.remote_url(&o.target_name(file))?;
    on_event(Event::Log(format!("Uploading {} ({}) to {url}", file.display(), human_size(size))));

    let mut child = Command::new("curl")
        .args(["-q", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "The curl program was not found. Uploading needs curl (with FTP/SFTP support) on the PATH."
                    .to_string()
            } else {
                format!("Cannot start curl: {e}")
            }
        })?;

    let mut stdin = child.stdin.take().ok_or("Cannot talk to curl.")?;
    stdin.write_all(o.curl_config(&url, file).as_bytes()).await.map_err(|e| format!("Cannot talk to curl: {e}"))?;
    drop(stdin); // curl starts when it has read the whole config

    let mut stderr = child.stderr.take().ok_or("Cannot read curl's output.")?;
    let mut pending: Vec<u8> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = stderr.read(&mut buf).await.map_err(|e| format!("Cannot read curl's output: {e}"))?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                handle_chunk(&mut pending, &mut on_event, &mut errors);
            } else {
                pending.push(b);
            }
        }
    }
    handle_chunk(&mut pending, &mut on_event, &mut errors);

    let status = child.wait().await.map_err(|e| format!("Cannot wait for curl: {e}"))?;
    if status.success() {
        on_event(Event::Progress(100.0));
        on_event(Event::Log("Upload finished.".to_string()));
        return Ok(());
    }

    let mut message = errors.last().cloned().unwrap_or_else(|| format!("curl stopped with {status}."));
    if message.contains("(60)") {
        message.push_str(" Add the server to your known hosts, or switch on \"Skip certificate / host key check\".");
    }
    Err(message)
}

/// Runs `command` through the shell. `{file}` in it becomes the (quoted) path of the output file, which is
/// also in the RXMLTV_OUTPUT environment variable. Each line the command prints goes to `on_line`.
pub async fn run_command(command: &str, file: &Path, mut on_line: impl FnMut(String)) -> Result<(), String> {
    let file_text = file.to_string_lossy().to_string();
    let quoted = if cfg!(windows) { format!("\"{file_text}\"") } else { crate::config::quote(&file_text) };
    let command = command.replace("{file}", &quoted);

    let mut shell = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(format!("{command} 2>&1"));
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(format!("exec 2>&1; {command}"));
        c
    };
    shell
        .env("RXMLTV_OUTPUT", &file_text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = shell.spawn().map_err(|e| format!("Cannot start the command: {e}"))?;
    let stdout = child.stdout.take().ok_or("Cannot read the command's output.")?;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await.map_err(|e| format!("Cannot read the command's output: {e}"))? {
        on_line(line);
    }

    let status = child.wait().await.map_err(|e| format!("Cannot wait for the command: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("The command stopped with {status}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(protocol: Protocol) -> UploadOptions {
        UploadOptions { protocol, host: "example.com".to_string(), ..UploadOptions::default() }
    }

    #[test]
    fn urls_for_every_protocol() {
        let mut o = options(Protocol::Ftp);
        assert_eq!(o.remote_url("epg.xml.gz").unwrap(), "ftp://example.com/epg.xml.gz");

        o.folder = "web/epg".to_string();
        o.port = Some(2121);
        assert_eq!(o.remote_url("epg.xml.gz").unwrap(), "ftp://example.com:2121/web/epg/epg.xml.gz");

        o.folder = "/srv/epg/".to_string(); // absolute on FTP: curl wants a double slash
        o.port = None;
        assert_eq!(o.remote_url("epg.xml.gz").unwrap(), "ftp://example.com//srv/epg/epg.xml.gz");

        let mut o = options(Protocol::Sftp);
        o.folder = "epg".to_string();
        assert_eq!(o.remote_url("a.xml").unwrap(), "sftp://example.com/~/epg/a.xml");
        o.folder = "/var/www".to_string();
        assert_eq!(o.remote_url("a.xml").unwrap(), "sftp://example.com/var/www/a.xml");

        let mut o = options(Protocol::WebDavs);
        o.folder = "/remote.php/dav/files/jan".to_string();
        assert_eq!(o.remote_url("a b.xml").unwrap(), "https://example.com/remote.php/dav/files/jan/a%20b.xml");
        o.protocol = Protocol::WebDav;
        assert!(o.remote_url("a.xml").unwrap().starts_with("http://"));
    }

    #[test]
    fn ipv6_and_missing_host() {
        let mut o = options(Protocol::Ftp);
        o.host = "::1".to_string();
        assert_eq!(o.remote_url("a.xml").unwrap(), "ftp://[::1]/a.xml");
        o.host = "  ".to_string();
        assert!(o.remote_url("a.xml").is_err());
    }

    #[test]
    fn the_target_name_defaults_to_the_output_name() {
        let mut o = options(Protocol::Ftp);
        assert_eq!(o.target_name(Path::new("/tmp/epg.xml.gz")), "epg.xml.gz");
        o.remote_name = "guide.xml".to_string();
        assert_eq!(o.target_name(Path::new("/tmp/epg.xml.gz")), "guide.xml");
    }

    #[test]
    fn secrets_go_into_the_curl_config_not_the_url() {
        let mut o = options(Protocol::Ftps);
        o.username = "jan".to_string();
        o.password = "p\"ss:word".to_string();
        let url = o.remote_url("a.xml").unwrap();
        assert!(!url.contains("jan") && !url.contains("ss"));

        let config = o.curl_config(&url, Path::new("/tmp/a.xml"));
        assert!(config.contains("user = \"jan:p\\\"ss:word\""));
        assert!(config.contains("ssl-reqd"));
        assert!(config.contains("upload-file = \"/tmp/a.xml\""));
        assert!(!config.contains("insecure"));
        o.insecure = true;
        assert!(o.curl_config(&url, Path::new("/tmp/a.xml")).contains("insecure"));
    }

    #[test]
    fn sftp_uses_the_key_and_its_passphrase() {
        let mut o = options(Protocol::Sftp);
        o.username = "jan".to_string();
        o.key_file = "/home/jan/.ssh/id_ed25519".to_string();
        o.key_passphrase = "secret".to_string();
        let config = o.curl_config("sftp://example.com/~/a.xml", Path::new("a.xml"));
        assert!(config.contains("key = \"/home/jan/.ssh/id_ed25519\""));
        assert!(config.contains("pass = \"secret\""));

        let mut ftp = options(Protocol::Ftp);
        ftp.key_file = "/ignored".to_string();
        assert!(!ftp.curl_config("ftp://example.com/a.xml", Path::new("a.xml")).contains("key ="));
    }

    #[test]
    fn curl_progress_is_understood() {
        assert_eq!(parse_percent("######## 45.2%"), Some(45.2));
        assert_eq!(parse_percent("100.0%"), Some(100.0));
        assert_eq!(parse_percent("curl: (67) Access denied"), None);
    }

    #[test]
    fn protocols_cycle() {
        assert_eq!(Protocol::Ftp.step(1), Protocol::Ftps);
        assert_eq!(Protocol::Ftp.step(-1), Protocol::WebDavs);
        assert_eq!(Protocol::parse(" SFTP "), Some(Protocol::Sftp));
        assert_eq!(Protocol::parse("scp"), None);
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 kB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }
}
