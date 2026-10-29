//! The updater's settings as a file (`rxmltv.conf`), as a command line, and the helpers the terminal UI uses
//! to change the group and channel selection.
#![cfg_attr(not(feature = "tui"), allow(dead_code))]

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use crate::groups::normalize;
use crate::util::with_suffix;
use crate::{ChannelRequest, Options};

pub const DEFAULT_PATH: &str = "rxmltv.conf";

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
    s
}

/// Written under a temporary name first, so a crash never leaves a half-written settings file.
pub fn save(path: &Path, o: &Options) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, to_text(o))?;
    fs::rename(&tmp, path)
}

fn quote(s: &str) -> String {
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
    for (on, flag) in [
        (o.clean_cache, "--clean-cache"),
        (o.strip_hd_tv, "--strip-hd-tv"),
        (o.keep_title_numbers, "--keep-title-numbers"),
        (o.keep_empty_channels, "--keep-empty-channels"),
        (o.no_images, "--no-images"),
        (o.retry_missing, "--retry-missing"),
        (o.refresh_channels, "--refresh-channels"),
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
}
