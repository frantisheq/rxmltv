//! The updater's settings as a file (`rxmltv.conf`), as a command line, and the helpers the terminal UI uses
//! to change the group and channel selection.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::groups::normalize;
use crate::upload::Protocol;
use crate::util::with_suffix;
use crate::{ChannelRequest, Options};

/// The hidden folder where rxmltv keeps what it creates: ~/.rxmltv (the settings file, the favorites, the
/// channel order and the cache).
pub fn app_dir() -> PathBuf {
    crate::util::home_dir().map(|home| home.join(".rxmltv")).unwrap_or_else(|| PathBuf::from(".rxmltv"))
}

/// Where the terminal UI keeps the settings unless told otherwise.
pub fn default_path() -> PathBuf {
    app_dir().join("rxmltv.conf")
}

pub fn default_cache_dir() -> String {
    app_dir().join("cache").to_string_lossy().to_string()
}

// ---------- colours (for the terminal UI's dots) ----------

pub const COLOR_NAMES: [&str; 21] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "gray", "darkgray", "lightred", "lightgreen",
    "lightyellow", "lightblue", "lightmagenta", "lightcyan", "white", "orange", "pink", "purple", "brown", "teal",
];

/// A colour name from COLOR_NAMES, or #rrggbb.
pub fn valid_color(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if COLOR_NAMES.contains(&t.as_str()) {
        return true;
    }
    t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// A genre keyword and the colour of its dot. The keyword is matched (without diacritics, any case) against
/// the programme's categories.
#[derive(Clone, Debug, PartialEq)]
pub struct GenreColor {
    pub keyword: String,
    pub color: String,
}

/// How the terminal UI looks.
#[derive(Clone, Debug)]
pub struct UiOptions {
    pub show_quality: bool,
    pub show_new: bool,
    pub show_rerun: bool,
    pub favorite_color: String,
    /// Show the guide as columns, one per channel, instead of a channel list and one programme list.
    pub grid_layout: bool,
    /// The first rule that matches wins.
    pub genre_colors: Vec<GenreColor>,
}

impl Default for UiOptions {
    fn default() -> Self {
        let rule = |keyword: &str, color: &str| GenreColor { keyword: keyword.to_string(), color: color.to_string() };
        UiOptions {
            show_quality: true,
            show_new: true,
            show_rerun: true,
            favorite_color: "orange".to_string(),
            grid_layout: false,
            genre_colors: vec![
                rule("serial", "blue"),
                rule("series", "blue"),
                rule("film", "red"),
                rule("movie", "red"),
                rule("zpravy", "yellow"),
                rule("news", "yellow"),
                rule("sport", "green"),
                rule("dokument", "cyan"),
                rule("documentar", "cyan"),
                rule("deti", "magenta"),
                rule("children", "magenta"),
                rule("kids", "magenta"),
                rule("hudba", "lightblue"),
                rule("music", "lightblue"),
                rule("zabav", "gray"),
                rule("entertain", "gray"),
            ],
        }
    }
}

impl UiOptions {
    /// The dot colour for a programme whose categories, normalized and joined, are `genres`.
    pub fn genre_color(&self, genres: &str) -> Option<&str> {
        self.genre_colors
            .iter()
            .find(|g| {
                let keyword = normalize(&g.keyword);
                !keyword.is_empty() && genres.contains(&keyword)
            })
            .map(|g| g.color.as_str())
    }
}

fn at(line: usize, message: String) -> String {
    format!("line {line}: {message}")
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

pub fn load(path: &Path) -> Result<Options, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// One `key = value` per line; `#` starts a comment line. `channel` may repeat.
pub fn parse(text: &str) -> Result<Options, String> {
    let mut o = Options::default();
    let mut genres_replaced = false; // the first genre_color line replaces the built-in rules

    for (index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(at(number, "expected \"key = value\".".to_string()));
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let flag = |name: &str| parse_bool(value).ok_or_else(|| at(number, format!("{name} must be true or false.")));
        let text_value = |name: &str| {
            if value.is_empty() {
                Err(at(number, format!("{name} must not be empty.")))
            } else {
                Ok(value.to_string())
            }
        };

        match key.as_str() {
            "days" => o.days = crate::parse_positive("days", value).map_err(|m| at(number, m))?,
            "concurrency" => {
                o.concurrency = crate::parse_positive("concurrency", value).map_err(|m| at(number, m))? as usize
            }
            "output" => o.output = text_value("output")?,
            "cache_dir" => o.cache_dir = text_value("cache_dir")?,
            "groups" => {
                let list: Vec<String> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(String::from)
                    .collect();
                o.groups = if list.is_empty() { None } else { Some(list) };
            }
            "channel" => o.channels.push(ChannelRequest::parse(value).map_err(|m| at(number, m))?),
            "clean_cache" => o.clean_cache = flag("clean_cache")?,
            "strip_hd_tv" => o.strip_hd_tv = flag("strip_hd_tv")?,
            "keep_title_numbers" => o.keep_title_numbers = flag("keep_title_numbers")?,
            "keep_empty_channels" => o.keep_empty_channels = flag("keep_empty_channels")?,
            "no_images" => o.no_images = flag("no_images")?,
            "retry_missing" => o.retry_missing = flag("retry_missing")?,
            "refresh_channels" => o.refresh_channels = flag("refresh_channels")?,
            "compress_cache" => o.compress_cache = flag("compress_cache")?,
            "show_quality" => o.ui.show_quality = flag("show_quality")?,
            "show_new" => o.ui.show_new = flag("show_new")?,
            "show_rerun" => o.ui.show_rerun = flag("show_rerun")?,
            "favorite_color" => {
                if !valid_color(value) {
                    return Err(at(number, format!("\"{value}\" is not a colour (a name like red, or #rrggbb).")));
                }
                o.ui.favorite_color = value.to_string();
            }
            "guide_layout" => {
                o.ui.grid_layout = match value.to_ascii_lowercase().as_str() {
                    "grid" => true,
                    "list" => false,
                    _ => return Err(at(number, "guide_layout must be list or grid.".to_string())),
                };
            }
            "genre_colors" => {
                // "none" means no rules at all
                o.ui.genre_colors.clear();
                genres_replaced = true;
            }
            "genre_color" => {
                let Some((keyword, color)) = value.split_once('=') else {
                    return Err(at(number, "expected \"genre_color = keyword = colour\".".to_string()));
                };
                let (keyword, color) = (keyword.trim(), color.trim());
                if keyword.is_empty() || !valid_color(color) {
                    return Err(at(number, format!("\"{value}\": needs a keyword and a colour (red, #rrggbb ...).")));
                }
                if !genres_replaced {
                    o.ui.genre_colors.clear();
                    genres_replaced = true;
                }
                o.ui.genre_colors.push(GenreColor { keyword: keyword.to_string(), color: color.to_string() });
            }
            "post_command" => o.post_command = value.to_string(),
            "upload_after_update" => o.upload.enabled = flag("upload_after_update")?,
            "upload_protocol" => {
                o.upload.protocol = Protocol::parse(value)
                    .ok_or_else(|| at(number, "upload_protocol must be ftp, ftps, sftp, webdav or webdavs.".to_string()))?;
            }
            "upload_host" => o.upload.host = value.to_string(),
            "upload_port" => {
                o.upload.port = if value.is_empty() {
                    None
                } else {
                    Some(value.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| {
                        at(number, "upload_port must be a number from 1 to 65535.".to_string())
                    })?)
                };
            }
            "upload_user" => o.upload.username = value.to_string(),
            "upload_password" => o.upload.password = value.to_string(),
            "upload_folder" => o.upload.folder = value.to_string(),
            "upload_name" => o.upload.remote_name = value.to_string(),
            "upload_key" => o.upload.key_file = value.to_string(),
            "upload_key_passphrase" => o.upload.key_passphrase = value.to_string(),
            "upload_insecure" => o.upload.insecure = flag("upload_insecure")?,
            other => return Err(at(number, format!("unknown setting \"{other}\"."))),
        }
    }

    Ok(o)
}

fn request_text(r: &ChannelRequest) -> String {
    match &r.new_id {
        Some(new_id) => format!("{} = {}", r.id, new_id),
        None => r.id.clone(),
    }
}

pub fn to_text(o: &Options) -> String {
    let mut s = String::new();
    s.push_str("# rxmltv settings. Written by `rxmltv --tui`, used with `rxmltv --config <this file>`.\n");
    s.push_str("# One \"key = value\" per line; lines starting with # are comments; \"channel\" may repeat.\n\n");
    let _ = writeln!(s, "days = {}", o.days);
    let _ = writeln!(s, "output = {}", o.output);
    let _ = writeln!(s, "cache_dir = {}", o.cache_dir);
    let _ = writeln!(s, "concurrency = {}", o.concurrency);
    let _ = writeln!(s, "groups = {}", o.groups.as_ref().map(|g| g.join(", ")).unwrap_or_default());
    for c in &o.channels {
        let _ = writeln!(s, "channel = {}", request_text(c));
    }
    let _ = writeln!(s, "clean_cache = {}", o.clean_cache);
    let _ = writeln!(s, "strip_hd_tv = {}", o.strip_hd_tv);
    let _ = writeln!(s, "keep_title_numbers = {}", o.keep_title_numbers);
    let _ = writeln!(s, "keep_empty_channels = {}", o.keep_empty_channels);
    let _ = writeln!(s, "no_images = {}", o.no_images);
    let _ = writeln!(s, "retry_missing = {}", o.retry_missing);
    let _ = writeln!(s, "refresh_channels = {}", o.refresh_channels);
    let _ = writeln!(s, "compress_cache = {}", o.compress_cache);

    let _ = writeln!(s, "\n# Terminal UI");
    let _ = writeln!(s, "show_quality = {}", o.ui.show_quality);
    let _ = writeln!(s, "show_new = {}", o.ui.show_new);
    let _ = writeln!(s, "show_rerun = {}", o.ui.show_rerun);
    let _ = writeln!(s, "favorite_color = {}", o.ui.favorite_color);
    let _ = writeln!(s, "guide_layout = {}", if o.ui.grid_layout { "grid" } else { "list" });
    if o.ui.genre_colors.is_empty() {
        let _ = writeln!(s, "genre_colors = none");
    }
    for g in &o.ui.genre_colors {
        let _ = writeln!(s, "genre_color = {} = {}", g.keyword, g.color);
    }

    let _ = writeln!(s, "\n# After an update. {{file}} in the command is the output file.");
    let _ = writeln!(s, "post_command = {}", o.post_command);
    let _ = writeln!(s, "\n# Upload (needs curl). Leave upload_password empty to use $RXMLTV_UPLOAD_PASSWORD.");
    let _ = writeln!(s, "upload_after_update = {}", o.upload.enabled);
    let _ = writeln!(s, "upload_protocol = {}", o.upload.protocol.name());
    let _ = writeln!(s, "upload_host = {}", o.upload.host);
    let _ = writeln!(s, "upload_port = {}", o.upload.port.map(|p| p.to_string()).unwrap_or_default());
    let _ = writeln!(s, "upload_user = {}", o.upload.username);
    let _ = writeln!(s, "upload_password = {}", o.upload.password);
    let _ = writeln!(s, "upload_folder = {}", o.upload.folder);
    let _ = writeln!(s, "upload_name = {}", o.upload.remote_name);
    let _ = writeln!(s, "upload_key = {}", o.upload.key_file);
    let _ = writeln!(s, "upload_key_passphrase = {}", o.upload.key_passphrase);
    let _ = writeln!(s, "upload_insecure = {}", o.upload.insecure);
    s
}

/// Written under a temporary name first, so a crash never leaves a half-written settings file.
pub fn save(path: &Path, o: &Options) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, to_text(o))?;
    #[cfg(unix)]
    if !o.upload.password.is_empty() || !o.upload.key_passphrase.is_empty() {
        // the file holds a password: only its owner may read it
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)
}

pub(crate) fn quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=,+@%".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The settings as a command line (only what differs from the defaults), e.g. for a cron job.
pub fn command_line(o: &Options) -> String {
    let d = Options::default();
    let mut parts: Vec<String> = vec!["rxmltv".to_string()];

    if o.days != d.days {
        parts.push(format!("--days {}", o.days));
    }
    if o.output != d.output {
        parts.push(format!("--output {}", quote(&o.output)));
    }
    if o.cache_dir != d.cache_dir {
        parts.push(format!("--cache-dir {}", quote(&o.cache_dir)));
    }
    if o.concurrency != d.concurrency {
        parts.push(format!("--concurrency {}", o.concurrency));
    }
    if let Some(groups) = &o.groups {
        parts.push(format!("--groups {}", quote(&groups.join(","))));
    }
    if !o.channels.is_empty() {
        let list: Vec<String> = o
            .channels
            .iter()
            .map(|c| match &c.new_id {
                Some(new_id) => format!("{}={}", c.id, new_id),
                None => c.id.clone(),
            })
            .collect();
        parts.push(format!("--channels {}", quote(&list.join(","))));
    }
    if o.upload.enabled {
        parts.push("--upload".to_string());
    }
    if !o.post_command.trim().is_empty() {
        parts.push(format!("--post-command {}", quote(o.post_command.trim())));
    }
    for (on, flag) in [
        (o.clean_cache, "--clean-cache"),
        (o.strip_hd_tv, "--strip-hd-tv"),
        (o.keep_title_numbers, "--keep-title-numbers"),
        (o.keep_empty_channels, "--keep-empty-channels"),
        (o.no_images, "--no-images"),
        (o.retry_missing, "--retry-missing"),
        (o.refresh_channels, "--refresh-channels"),
        (o.compress_cache, "--compress-cache"),
    ] {
        if on {
            parts.push(flag.to_string());
        }
    }

    parts.join(" ")
}

/// "all channels", "groups cz, sk", "groups cz + 3 channels", "3 channels".
pub fn selection_summary(o: &Options) -> String {
    match (&o.groups, o.channels.len()) {
        (None, 0) => "all channels".to_string(),
        (None, n) => format!("{n} channel(s)"),
        (Some(g), 0) => format!("groups {}", g.join(", ")),
        (Some(g), n) => format!("groups {} + {n} channel(s)", g.join(", ")),
    }
}

// ---------- selection helpers (used by the terminal UI's channel picker) ----------

/// Does the entry the user typed or saved ("cz", "České", "ceske") mean this group?
pub fn group_matches(selected: &str, code: &str, name: &str) -> bool {
    let n = normalize(selected);
    n == code || n == normalize(name)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn group_selected(o: &Options, code: &str, name: &str) -> bool {
    o.groups.as_ref().is_some_and(|g| g.iter().any(|x| group_matches(x, code, name)))
}

pub fn toggle_group(o: &mut Options, code: &str, name: &str) {
    let mut list = o.groups.take().unwrap_or_default();
    if list.iter().any(|x| group_matches(x, code, name)) {
        list.retain(|x| !group_matches(x, code, name));
    } else {
        list.push(code.to_string());
    }
    o.groups = if list.is_empty() { None } else { Some(list) };
}

pub fn channel_request<'a>(o: &'a Options, xml_id: &str) -> Option<&'a ChannelRequest> {
    o.channels.iter().find(|r| r.id.eq_ignore_ascii_case(xml_id))
}

pub fn toggle_channel(o: &mut Options, xml_id: &str) {
    match o.channels.iter().position(|r| r.id.eq_ignore_ascii_case(xml_id)) {
        Some(pos) => {
            o.channels.remove(pos);
        }
        None => o.channels.push(ChannelRequest { id: xml_id.to_string(), new_id: None }),
    }
}

/// Sets (or, with None, removes) the id a channel is written under. Picks the channel if it wasn't picked yet.
pub fn set_new_id(o: &mut Options, xml_id: &str, new_id: Option<String>) {
    match o.channels.iter_mut().find(|r| r.id.eq_ignore_ascii_case(xml_id)) {
        Some(r) => r.new_id = new_id,
        None => o.channels.push(ChannelRequest { id: xml_id.to_string(), new_id }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip() {
        let mut o = Options::default();
        o.days = 3;
        o.output = "/var/www/epg/guide.xml.gz".to_string();
        o.concurrency = 8;
        o.groups = Some(vec!["cz".to_string(), "sk".to_string()]);
        o.channels = vec![
            ChannelRequest { id: "cz-nova-2".into(), new_id: Some("nova.cz".into()) },
            ChannelRequest { id: "en-bbc-9".into(), new_id: None },
        ];
        o.clean_cache = true;
        o.no_images = true;

        let back = parse(&to_text(&o)).unwrap();
        assert_eq!(back.days, 3);
        assert_eq!(back.output, o.output);
        assert_eq!(back.concurrency, 8);
        assert_eq!(back.groups, o.groups);
        assert_eq!(back.channels, o.channels);
        assert!(back.clean_cache && back.no_images && !back.strip_hd_tv);
    }

    #[test]
    fn empty_groups_mean_all_groups() {
        let o = parse("groups =\n").unwrap();
        assert_eq!(o.groups, None);
    }

    #[test]
    fn mistakes_name_the_line() {
        assert!(parse("days = 0").unwrap_err().contains("line 1"));
        assert!(parse("\nnonsense").unwrap_err().contains("line 2"));
        assert!(parse("colour = red").unwrap_err().contains("unknown setting"));
        assert!(parse("no_images = maybe").unwrap_err().contains("true or false"));
        assert!(parse("output =").unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn comments_and_bom_are_ignored() {
        let o = parse("\u{feff}# hello\n\ndays = 2\n").unwrap();
        assert_eq!(o.days, 2);
    }

    #[test]
    fn command_line_shows_only_changes() {
        assert_eq!(command_line(&Options::default()), "rxmltv");
        let mut o = Options::default();
        o.days = 2;
        o.groups = Some(vec!["cz".into(), "sk".into()]);
        o.output = "my guide.xml".into();
        o.clean_cache = true;
        o.channels = vec![ChannelRequest { id: "en-bbc-9".into(), new_id: Some("bbc.uk".into()) }];
        assert_eq!(
            command_line(&o),
            "rxmltv --days 2 --output 'my guide.xml' --groups cz,sk --channels en-bbc-9=bbc.uk --clean-cache"
        );
    }

    #[test]
    fn groups_toggle_by_code_or_name() {
        let mut o = Options::default();
        toggle_group(&mut o, "cz", "České");
        assert_eq!(o.groups, Some(vec!["cz".to_string()]));
        assert!(group_selected(&o, "cz", "České"));
        toggle_group(&mut o, "sk", "Slovenské");
        assert_eq!(o.groups, Some(vec!["cz".to_string(), "sk".to_string()]));

        // a group given by name on the command line is recognised and removed again
        o.groups = Some(vec!["České".to_string()]);
        assert!(group_selected(&o, "cz", "České"));
        toggle_group(&mut o, "cz", "České");
        assert_eq!(o.groups, None);
    }

    #[test]
    fn channels_toggle_and_get_new_ids() {
        let mut o = Options::default();
        toggle_channel(&mut o, "cz-nova-2");
        assert_eq!(o.channels.len(), 1);
        set_new_id(&mut o, "CZ-NOVA-2", Some("nova.cz".into()));
        assert_eq!(channel_request(&o, "cz-nova-2").unwrap().new_id.as_deref(), Some("nova.cz"));
        set_new_id(&mut o, "sk-markiza-3", Some("m.sk".into())); // not picked yet: gets picked
        assert_eq!(o.channels.len(), 2);
        set_new_id(&mut o, "sk-markiza-3", None);
        assert_eq!(channel_request(&o, "sk-markiza-3").unwrap().new_id, None);
        toggle_channel(&mut o, "cz-nova-2");
        assert_eq!(o.channels.len(), 1);
    }

    #[test]
    fn selection_is_summarised() {
        let mut o = Options::default();
        assert_eq!(selection_summary(&o), "all channels");
        o.groups = Some(vec!["cz".into()]);
        assert_eq!(selection_summary(&o), "groups cz");
        o.channels = vec![ChannelRequest { id: "en-bbc-9".into(), new_id: None }];
        assert_eq!(selection_summary(&o), "groups cz + 1 channel(s)");
    }

    #[test]
    fn ui_and_upload_settings_survive_a_round_trip() {
        let mut o = Options::default();
        o.ui.show_rerun = false;
        o.ui.favorite_color = "#ff8800".to_string();
        o.ui.genre_colors = vec![GenreColor { keyword: "sport".into(), color: "green".into() }];
        o.post_command = "cp {file} /srv/epg.xml".to_string();
        o.upload.enabled = true;
        o.upload.protocol = Protocol::Sftp;
        o.upload.host = "example.com".to_string();
        o.upload.port = Some(2222);
        o.upload.username = "jan".to_string();
        o.upload.folder = "/var/www/epg".to_string();
        o.upload.key_file = "/home/jan/.ssh/id_ed25519".to_string();
        o.upload.insecure = true;

        let back = parse(&to_text(&o)).unwrap();
        assert!(!back.ui.show_rerun && back.ui.show_new);
        assert_eq!(back.ui.favorite_color, "#ff8800");
        assert_eq!(back.ui.genre_colors, o.ui.genre_colors);
        assert_eq!(back.post_command, o.post_command);
        assert!(back.upload.enabled && back.upload.insecure);
        assert_eq!(back.upload.protocol, Protocol::Sftp);
        assert_eq!(back.upload.port, Some(2222));
        assert_eq!(back.upload.key_file, o.upload.key_file);
    }

    #[test]
    fn the_guide_layout_is_saved() {
        let mut o = Options::default();
        assert!(!parse(&to_text(&o)).unwrap().ui.grid_layout);
        o.ui.grid_layout = true;
        assert!(parse(&to_text(&o)).unwrap().ui.grid_layout);
        assert!(parse("guide_layout = tiles").is_err());
    }

    #[test]
    fn genre_rules_can_be_emptied_and_default_otherwise() {
        assert!(!parse("days = 1").unwrap().ui.genre_colors.is_empty());
        let mut o = Options::default();
        o.ui.genre_colors.clear();
        assert!(parse(&to_text(&o)).unwrap().ui.genre_colors.is_empty());
    }

    #[test]
    fn bad_ui_and_upload_values_are_reported() {
        assert!(parse("favorite_color = chartreuse").is_err());
        assert!(parse("genre_color = sport").is_err());
        assert!(parse("genre_color = sport = nope").is_err());
        assert!(parse("upload_protocol = scp").is_err());
        assert!(parse("upload_port = 70000").is_err());
        assert_eq!(parse("upload_port =").unwrap().upload.port, None);
    }

    #[test]
    fn colours_are_checked() {
        assert!(valid_color("Red"));
        assert!(valid_color("#00FFaa"));
        assert!(!valid_color("#00ffa"));
        assert!(!valid_color("reddish"));
    }

    #[test]
    fn genres_pick_the_first_matching_rule() {
        let ui = UiOptions::default();
        assert_eq!(ui.genre_color("series akcni drama"), Some("blue"));
        assert_eq!(ui.genre_color("zabava"), Some("gray"));
        assert_eq!(ui.genre_color("something else"), None);
    }

    #[test]
    fn upload_and_command_show_in_the_command_line() {
        let mut o = Options::default();
        o.upload.enabled = true;
        o.post_command = "cp {file} /srv/epg.xml".to_string();
        assert_eq!(command_line(&o), "rxmltv --upload --post-command 'cp {file} /srv/epg.xml'");
    }
}
