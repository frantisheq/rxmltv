mod enrichment;
mod fetcher;
mod groups;
mod output;
mod stats;
mod util;

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, LocalResult, NaiveDate, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use roxmltree::Node;
use tokio::task::JoinHandle;

use crate::enrichment::{DetailData, ProgrammeInfo};
use crate::fetcher::Fetcher;
use crate::groups::{group_code, natural_cmp, normalize, unidecode};
use crate::util::{child, child_value, get_numbers, html_decode, is_el, key, value, with_suffix, TempGuard};

type BoxError = Box<dyn Error + Send + Sync>;

const USAGE: &str = r#"Usage: rxmltv [options]
  --days <n>          Number of days to download (default 7)
  --output <file>     Output XMLTV file (default xmltv.xml; a .gz name is written compressed)
  --cache-dir <dir>   Cache folder (default data/cache/)
  --clean-cache       Delete cached files older than today
  --concurrency <n>   Parallel downloads (default 4)
  --groups <a,b,c>    Only these channel groups, by name or code, e.g. "České,Slovenské" or "cz,sk" (default: all groups)
  --channels <a,b,c>  Pick individual channels by their XMLTV id, e.g. "cz-ct1-2,sk-markiza-7"
                      (see --list-channels). Combined with --groups, the result is both selections together
  --channels-file <f> Same as --channels, one id per line ("#" starts a comment, only the first word of a line is used)
  --list-groups       Print the groups the server offers, with channel counts, and exit
  --list-channels     Print every channel (XMLTV id and name) for use with --channels, and exit
  --keep-title-numbers   Leave "(3)", "(2/6)" and roman season numbers in the title
                         (default: removed and written as <episode-num> instead)
  --keep-empty-channels  Also write channels that have no programmes
  --no-images         Don't write poster <icon> elements
  --retry-missing     Ask the server again for items it had no data for (otherwise skipped for 12 h)
  -h, --help          Show this help"#;

#[derive(Debug, Clone)]
pub struct Options {
    pub days: u32,
    pub clean_cache: bool,
    pub output: String,
    pub cache_dir: String,
    pub concurrency: usize,
    pub groups: Option<Vec<String>>,
    pub channels: Vec<String>,
    pub list_groups: bool,
    pub list_channels: bool,
    pub keep_title_numbers: bool,
    pub keep_empty_channels: bool,
    pub no_images: bool,
    pub retry_missing: bool,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub logo: Option<String>,
    /// "<group code>-<name>-<id>", e.g. "cz-ct1-2". The code comes from the channel group (see groups.rs).
    pub xml_id: String,
}

impl Channel {
    pub fn new(id: String, name: String, logo: Option<String>, code: String) -> Self {
        let xml_id = format!("{}-{}-{}", code, unidecode(&name).to_lowercase().replace(' ', "-"), id);
        Channel { id, name, logo, xml_id }
    }
}

#[derive(Debug)]
pub struct Programme {
    pub start: String,
    pub stop: Option<String>,
    pub info: ProgrammeInfo,
}

// ---------- entry point ----------

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("Error: {e}\n");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    tokio::select! {
        result = run(opts) => match result {
            Ok(code) => ExitCode::from(code),
            Err(e) => {
                eprintln!("\nFatal: {e}");
                ExitCode::from(1)
            }
        },
        _ = shutdown_signal() => {
            eprintln!("\nCancelled.");
            ExitCode::from(130)
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ---------- args ----------

fn take_value(name: &str, inline: Option<&str>, args: &[String], i: &mut usize) -> Result<String, String> {
    if let Some(v) = inline {
        return Ok(v.to_string());
    }
    if *i + 1 < args.len() {
        *i += 1;
        return Ok(args[*i].clone());
    }
    Err(format!("{name} needs a value."))
}

fn parse_positive(name: &str, value: &str) -> Result<u32, String> {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = value.parse::<u32>() {
            if n > 0 {
                return Ok(n);
            }
        }
    }
    Err(format!("{name} must be a positive number."))
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut o = Options {
        days: 7,
        clean_cache: false,
        output: "xmltv.xml".to_string(),
        cache_dir: "data/cache/".to_string(),
        concurrency: 4,
        groups: None,
        channels: Vec::new(),
        list_groups: false,
        list_channels: false,
        keep_title_numbers: false,
        keep_empty_channels: false,
        no_images: false,
        retry_missing: false,
    };

    let mut i = 0;
    while i < args.len() {
        let mut name = args[i].as_str();
        let mut inline: Option<&str> = None;
        if name.starts_with("--") {
            if let Some(eq) = name.find('=') {
                if eq > 0 {
                    inline = Some(&name[eq + 1..]);
                    name = &name[..eq];
                }
            }
        }

        match name {
            "-h" | "--help" => return Ok(None),
            "--clean-cache" => o.clean_cache = true,
            "--list-groups" => o.list_groups = true,
            "--list-channels" => o.list_channels = true,
            "--keep-title-numbers" => o.keep_title_numbers = true,
            "--keep-empty-channels" => o.keep_empty_channels = true,
            "--no-images" => o.no_images = true,
            "--retry-missing" => o.retry_missing = true,
            "--groups" => {
                let v = take_value(name, inline, args, &mut i)?;
                let groups: Vec<String> = v
                    .split(',')
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(String::from)
                    .collect();
                if groups.is_empty() {
                    return Err("--groups needs at least one name.".to_string());
                }
                o.groups = Some(groups);
            }
            "--channels" => {
                let v = take_value(name, inline, args, &mut i)?;
                o.channels.extend(v.split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from));
            }
            "--channels-file" => {
                let path = take_value(name, inline, args, &mut i)?;
                let text = fs::read_to_string(&path).map_err(|e| format!("Cannot read {path}: {e}"))?;
                o.channels.extend(
                    text.trim_start_matches('\u{feff}')
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.starts_with('#'))
                        .filter_map(|l| l.split_whitespace().next())
                        .map(String::from),
                );
            }
            "--days" => o.days = parse_positive(name, &take_value(name, inline, args, &mut i)?)?,
            "--concurrency" => {
                o.concurrency = parse_positive(name, &take_value(name, inline, args, &mut i)?)? as usize
            }
            "--output" => o.output = take_value(name, inline, args, &mut i)?,
            "--cache-dir" => o.cache_dir = take_value(name, inline, args, &mut i)?,
            _ => return Err(format!("Unknown option: {name}")),
        }
        i += 1;
    }

    Ok(Some(o))
}

// ---------- channels ----------

struct RawChannel {
    id: Option<String>,
    name: Option<String>,
    logo: Option<String>,
    group: String,
}

fn parse_channel_list(root: Node) -> Vec<RawChannel> {
    root.children()
        .filter(|c| is_el(c, "a"))
        .map(|a| RawChannel {
            id: a.attribute("id").map(String::from),
            name: child_value(a, "n"),
            logo: child_value(a, "o"),
            group: child_value(a, "p").unwrap_or_default().trim().to_string(),
        })
        .collect()
}

// Channel ids become part of cache file names, so only accept safe ones.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Which channels to produce: everything when no filter is given, otherwise the channels of the
/// requested groups plus the individually requested ones (by XMLTV id, ignoring case).
fn parse_channels(raw: &[RawChannel], groups: Option<&[String]>, channels: &[String]) -> Vec<Arc<Channel>> {
    let wanted_groups: Option<HashSet<String>> = groups.map(|g| g.iter().map(|x| normalize(x)).collect());
    let wanted_ids: HashSet<String> = channels.iter().map(|c| c.trim().to_lowercase()).collect();
    let filtering = wanted_groups.is_some() || !wanted_ids.is_empty();
    let mut found: HashSet<String> = HashSet::new();

    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<(Channel, String, String)> = Vec::new(); // (channel, code, sort key)

    for r in raw {
        let (Some(id), Some(name)) = (&r.id, &r.name) else { continue };
        if !is_safe_id(id) {
            continue;
        }
        let code = group_code(Some(&r.group));
        let channel = Channel::new(id.clone(), name.clone(), r.logo.clone(), code.clone());

        if filtering {
            let group_n = normalize(&r.group);
            let in_group = wanted_groups
                .as_ref()
                .is_some_and(|w| w.contains(&group_n) || w.contains(&code));

            let xml_id = channel.xml_id.to_lowercase();
            let picked = wanted_ids.contains(&xml_id);
            if picked {
                found.insert(xml_id);
            }
            if !(in_group || picked) {
                continue;
            }
        }

        if !seen.insert(id.clone()) {
            continue; // a channel listed in two groups must only appear once in the XMLTV
        }
        items.push((channel, code, unidecode(name)));
    }

    for wanted in channels {
        if !found.contains(&wanted.trim().to_lowercase()) {
            eprintln!("Warning: channel \"{wanted}\" not found (see --list-channels).");
        }
    }

    // Countries alphabetically by code, then channels alphabetically ignoring diacritics and case.
    items.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| natural_cmp(&a.2, &b.2)));
    items.into_iter().map(|(c, _, _)| Arc::new(c)).collect()
}

fn warn_about_unknown_groups(raw: &[RawChannel], requested: Option<&[String]>) {
    let Some(requested) = requested else { return };
    let available: HashSet<String> = raw
        .iter()
        .flat_map(|r| [normalize(&r.group), group_code(Some(&r.group))])
        .collect();
    for g in requested.iter().filter(|g| !available.contains(&normalize(g))) {
        eprintln!("Warning: group \"{g}\" not found (see --list-groups).");
    }
}

fn print_groups(raw: &[RawChannel]) {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for r in raw {
        match groups.iter_mut().find(|(name, _)| *name == r.group) {
            Some(entry) => entry.1 += 1,
            None => groups.push((r.group.clone(), 1)),
        }
    }
    groups.sort_by(|a, b| b.1.cmp(&a.1)); // stable: ties keep the server's order

    for (name, count) in &groups {
        let shown = if name.is_empty() { "(none)" } else { name.as_str() };
        println!("{:>5}  {}  [{}]", count, shown, group_code(Some(name)));
    }
}

/// The XMLTV id (as written to the output) and the name of every channel, in output order.
fn print_channels(raw: &[RawChannel]) {
    let channels = parse_channels(raw, None, &[]);
    let width = channels.iter().map(|c| c.xml_id.chars().count()).max().unwrap_or(0);
    for c in &channels {
        println!("{:<width$}  {}", c.xml_id, c.name);
    }
}

// ---------- loading ----------

/// One entry of a day listing, copied out of the XML.
struct DayEntry {
    start: Option<String>,
    stop: Option<String>,
    title: Option<String>,
    clean_title: Option<String>,
    short: Option<String>,
}

impl DayEntry {
    fn from_node(p: Node) -> Self {
        let n = child(p, "n");
        DayEntry {
            start: p.attribute("o").map(String::from),
            stop: p.attribute("d").map(String::from),
            title: n.map(value),
            // <n u="Series name"> is the title without season/episode numbers.
            clean_title: n.and_then(|n| n.attribute("u")).map(String::from),
            short: child_value(p, "k"),
        }
    }
}

async fn load_channel(fetcher: &Fetcher, ch: &Channel, today: NaiveDate, days: u32, tz: Tz) -> Vec<Programme> {
    let mut result = Vec::new();
    let mut seen: HashSet<String> = HashSet::new(); // a show can appear in two neighbouring day files; keep one per start time

    // Days and programmes are processed one after another on purpose: parallelism comes from the
    // channels, and this keeps only a handful of parsed files in memory at any moment.
    for i in 0..days {
        let day = today + Duration::days(i64::from(i));
        for p in load_day(fetcher, ch, day, tz).await {
            if seen.insert(p.start.clone()) {
                result.push(p);
            }
        }
    }
    result
}

async fn load_day(fetcher: &Fetcher, ch: &Channel, day: NaiveDate, tz: Tz) -> Vec<Programme> {
    let d = day.format("%Y%m%d").to_string();
    let uri = format!("v5-program.php?datum={d}&id_tv={}", ch.id);
    let file = format!("{}-{d}.xml", ch.id);

    let entries = fetcher
        .get(&uri, &file, false, |root| {
            root.children()
                .filter(|c| is_el(c, "p"))
                .map(DayEntry::from_node)
                .collect::<Vec<_>>()
        })
        .await;
    let Some(entries) = entries else { return Vec::new() };

    let mut list = Vec::new();
    for entry in &entries {
        if let Some(p) = build_programme(fetcher, ch, entry, tz).await {
            list.push(p);
        }
    }
    list
}

async fn build_programme(fetcher: &Fetcher, ch: &Channel, entry: &DayEntry, tz: Tz) -> Option<Programme> {
    let list_start = entry.start.as_deref();
    let key = get_numbers(list_start.unwrap_or(""));

    // The detail file has the long description. If it can't be fetched, fall back
    // to what the day listing already told us instead of dropping the show.
    let mut detail: Option<DetailData> = None;
    if !key.is_empty() {
        let uri = format!("v5-porad.php?datum={key}&id_tv={}", ch.id);
        let file = format!("{}-{key}.xml", ch.id);

        detail = fetcher.get(&uri, &file, false, DetailData::from_node).await;

        // The detail file must describe THIS programme. If it doesn't (the schedule changed after it was
        // cached, or the server answered with the enclosing/neighbouring programme), its cast and roles
        // would be attached to the wrong show. Fetch it once more; if it is still wrong, ignore it.
        if detail.as_ref().is_some_and(|d| !belongs_to(d, entry, &key)) {
            stats::inc(&stats::STALE);
            detail = fetcher.get(&uri, &file, true, DetailData::from_node).await;

            if detail.as_ref().is_some_and(|d| !belongs_to(d, entry, &key)) {
                fetcher.reject(&file);
                stats::inc(&stats::REJECTED);
                detail = None;
            }
        }
    }

    let start = detail.as_ref().and_then(|d| d.start.as_deref()).or(list_start);
    let stop = detail.as_ref().and_then(|d| d.stop.as_deref()).or(entry.stop.as_deref());
    let title = detail.as_ref().and_then(|d| d.title.as_deref()).or(entry.title.as_deref());

    let start_fmt = start.and_then(|s| format_time(s, tz))?;
    let title = title.filter(|t| !t.trim().is_empty())?;
    let mut stop_fmt = stop.and_then(|s| format_time(s, tz));

    let started = Instant::now();
    let info = enrichment::extract(title, entry.clean_title.as_deref(), detail.as_ref(), entry.short.as_deref());
    stats::add_since(&stats::EXTRACT_NANOS, started);

    if stop_fmt.as_deref() == Some(start_fmt.as_str()) {
        stop_fmt = None; // a few zero-length entries exist
    }

    Some(Programme { start: start_fmt, stop: stop_fmt, info })
}

/// True when the detail file is for the same programme as the day-listing entry.
fn belongs_to(detail: &DetailData, entry: &DayEntry, key: &str) -> bool {
    // Same start time? (the key is the listing's start with everything but digits removed)
    if let Some(detail_start) = &detail.start {
        if get_numbers(detail_start) != key {
            return false;
        }
    }

    // Similar title? Lenient on purpose ("New: Foo (5)" vs "Foo (5)"), strict enough to catch another show.
    let a = title_key(detail.title.as_deref());
    let b = title_key(entry.title.as_deref());
    if a.chars().count() < 3 || b.chars().count() < 3 {
        return true; // nothing to compare
    }
    a.contains(&b) || b.contains(&a)
}

fn title_key(title: Option<&str>) -> String {
    title.map(|t| key(&html_decode(t))).unwrap_or_default()
}

// ---------- time ----------

/// Turns "2026-10-18 20:15:00" into "20261018201500 +0200" using the real offset for that date.
fn format_time(raw: &str, tz: Tz) -> Option<String> {
    let digits = get_numbers(raw);
    if digits.len() != 14 {
        return None;
    }
    let n = |range: std::ops::Range<usize>| digits[range].parse::<u32>().ok();

    let date = NaiveDate::from_ymd_opt(digits[0..4].parse().ok()?, n(4..6)?, n(6..8)?)?;
    let local = date.and_hms_opt(n(8..10)?, n(10..12)?, n(12..14)?)?;

    // Like .NET: an ambiguous time (clocks go back) and a non-existent one (clocks go forward)
    // both get the standard offset.
    let seconds = match tz.offset_from_local_datetime(&local) {
        LocalResult::Single(o) => o.fix().local_minus_utc(),
        LocalResult::Ambiguous(_, later) => later.fix().local_minus_utc(),
        LocalResult::None => 3600,
    };

    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.abs();
    Some(format!("{digits} {sign}{:02}{:02}", abs / 3600, (abs % 3600) / 60))
}

fn format_duration(t: std::time::Duration) -> String {
    let secs = t.as_secs();
    if secs >= 3600 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

// ---------- run ----------

async fn run(opts: Options) -> Result<u8, BoxError> {
    let cache_dir = PathBuf::from(&opts.cache_dir);
    fs::create_dir_all(&cache_dir)?;

    let tz = chrono_tz::Europe::Prague;
    let today = Utc::now().with_timezone(&tz).date_naive();

    if opts.clean_cache {
        fetcher::clean_cache(&cache_dir, today);
    }

    let fetcher = Arc::new(Fetcher::new(&cache_dir, opts.concurrency, opts.retry_missing)?);

    let Some(raw) = fetcher.get("v5-tv.php?locale=cz", "channels.xml", false, parse_channel_list).await else {
        eprintln!("Could not load the channel list.");
        return Ok(1);
    };

    if opts.list_groups {
        print_groups(&raw);
        return Ok(0);
    }

    if opts.list_channels {
        print_channels(&raw);
        return Ok(0);
    }

    warn_about_unknown_groups(&raw, opts.groups.as_deref());

    let channels = parse_channels(&raw, opts.groups.as_deref(), &opts.channels);
    if channels.is_empty() {
        eprintln!("No channels matched. Run with --list-groups to see the available groups.");
        return Ok(1);
    }

    eprintln!("{} channels, {} day(s).", channels.len(), opts.days);

    let clock = Instant::now();

    let full = std::path::absolute(&opts.output)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)?;
    }
    let programmes_tmp = with_suffix(&full, ".programmes.tmp");
    let output_tmp = with_suffix(&full, ".tmp");
    let _guard_programmes = TempGuard(programmes_tmp.clone());
    let _guard_output = TempGuard(output_tmp.clone());

    let mut written: Vec<Arc<Channel>> = Vec::new();
    let mut programme_count = 0usize;

    {
        // Memory stays flat: only a few channels are loaded at a time, and the programmes of a channel are
        // serialised to a temporary file as soon as it is finished, so nothing piles up.
        let mut writer = BufWriter::new(File::create(&programmes_tmp)?);

        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let window = std::cmp::max(8, std::cmp::max(opts.concurrency * 2, cpus * 2));
        let mut pending: VecDeque<(Arc<Channel>, JoinHandle<Vec<Programme>>)> = VecDeque::new();
        let (mut next, mut done) = (0usize, 0usize);
        let mut buffer = String::new();

        while next < channels.len() || !pending.is_empty() {
            while next < channels.len() && pending.len() < window {
                let ch = Arc::clone(&channels[next]);
                next += 1;

                let task_fetcher = Arc::clone(&fetcher);
                let task_channel = Arc::clone(&ch);
                let days = opts.days;
                let handle = tokio::spawn(async move {
                    load_channel(&task_fetcher, &task_channel, today, days, tz).await
                });
                pending.push_back((ch, handle));
            }

            // Results are consumed in channel order, so the output is deterministic.
            let Some((channel, handle)) = pending.pop_front() else { break };
            let list = handle.await?;

            if !list.is_empty() || opts.keep_empty_channels {
                written.push(Arc::clone(&channel));

                let started = Instant::now();
                for p in &list {
                    buffer.clear();
                    output::write_programme(&mut buffer, p, &channel.xml_id, &opts);
                    writer.write_all(buffer.as_bytes())?;
                }
                stats::add_since(&stats::WRITE_NANOS, started);

                programme_count += list.len();
            }

            done += 1;
            let line = format!("\r({done}/{}) {}", channels.len(), channel.name);
            eprint!("{line:<70}");
        }

        writer.flush()?;
    }

    eprintln!();

    let started = Instant::now();
    output::assemble_output(&full, &output_tmp, &written, &programmes_tmp)?;
    stats::add_since(&stats::WRITE_NANOS, started);

    eprintln!(
        "Wrote {} programmes on {} channels to {} in {}.",
        stats::n0(programme_count as u64),
        written.len(),
        opts.output,
        format_duration(clock.elapsed())
    );
    stats::print();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn times_get_the_real_offset() {
        let tz = chrono_tz::Europe::Prague;
        assert_eq!(format_time("2026-10-18 20:15:00", tz).as_deref(), Some("20261018201500 +0200"));
        assert_eq!(format_time("2026-12-01 08:00:00", tz).as_deref(), Some("20261201080000 +0100"));
        assert_eq!(format_time("nonsense", tz), None);
        assert_eq!(format_time("2026-13-45 99:99:99", tz), None);
    }

    #[test]
    fn arguments_are_parsed() {
        let o = parse_args(&args(&["--days", "3", "--groups=cz, sk", "--no-images", "--output", "a.xmltv.gz"]))
            .unwrap()
            .unwrap();
        assert_eq!(o.days, 3);
        assert_eq!(o.groups, Some(vec!["cz".to_string(), "sk".to_string()]));
        assert!(o.no_images);
        assert_eq!(o.output, "a.xmltv.gz");
        assert_eq!(o.concurrency, 4);
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert!(parse_args(&args(&["--days", "0"])).is_err());
        assert!(parse_args(&args(&["--days"])).is_err());
        assert!(parse_args(&args(&["--nope"])).is_err());
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
    }

    fn raw_channel(id: &str, name: &str, group: &str) -> RawChannel {
        RawChannel {
            id: Some(id.to_string()),
            name: Some(name.to_string()),
            logo: None,
            group: group.to_string(),
        }
    }

    // XMLTV ids: cz-ct1-1, cz-nova-2, sk-markiza-3, cz-prima-zoom-4
    fn sample() -> Vec<RawChannel> {
        vec![
            raw_channel("1", "ČT1", "České"),
            raw_channel("2", "Nova", "České"),
            raw_channel("3", "Markíza", "Slovenské"),
            raw_channel("4", "Prima Zoom", "České"),
        ]
    }

    fn ids(list: &[Arc<Channel>]) -> Vec<&str> {
        list.iter().map(|c| c.id.as_str()).collect()
    }

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_filter_selects_everything() {
        assert_eq!(ids(&parse_channels(&sample(), None, &[])), vec!["1", "2", "4", "3"]);
    }

    #[test]
    fn channels_are_picked_by_xmltv_id() {
        assert_eq!(ids(&parse_channels(&sample(), None, &list(&["cz-nova-2"]))), vec!["2"]);
        assert_eq!(
            ids(&parse_channels(&sample(), None, &list(&["sk-markiza-3", "cz-ct1-1"]))),
            vec!["1", "3"]
        );
    }

    #[test]
    fn xmltv_ids_ignore_case() {
        assert_eq!(ids(&parse_channels(&sample(), None, &list(&["SK-Markiza-3"]))), vec!["3"]);
    }

    #[test]
    fn names_and_plain_ids_do_not_match() {
        assert!(parse_channels(&sample(), None, &list(&["Nova"])).is_empty());
        assert!(parse_channels(&sample(), None, &list(&["2"])).is_empty());
    }

    #[test]
    fn groups_and_channels_are_combined() {
        let groups = list(&["sk"]);
        let chosen = parse_channels(&sample(), Some(&groups), &list(&["cz-nova-2"]));
        assert_eq!(ids(&chosen), vec!["2", "3"]);
    }

    #[test]
    fn channels_option_is_parsed() {
        let o = parse_args(&args(&["--channels", "cz-nova-2, sk-markiza-3"])).unwrap().unwrap();
        assert_eq!(o.channels, vec!["cz-nova-2".to_string(), "sk-markiza-3".to_string()]);
    }

    #[test]
    fn channels_file_takes_the_first_word_of_each_line() {
        let path = std::env::temp_dir().join(format!("rxmltv-test-{}.txt", std::process::id()));
        fs::write(&path, "# my channels\ncz-nova-2   Nova\n\n  sk-markiza-3\n").unwrap();
        let o = parse_args(&args(&["--channels-file", path.to_str().unwrap()])).unwrap().unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(o.channels, vec!["cz-nova-2".to_string(), "sk-markiza-3".to_string()]);
    }

    #[test]
    fn channel_ids_must_be_safe_file_names() {
        assert!(is_safe_id("ct1_HD-2"));
        assert!(!is_safe_id("../x"));
        assert!(!is_safe_id(""));
    }
}
